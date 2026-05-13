//! Extra hook backends for Cline (executable script), Gemini CLI
//! (AfterTool JSON), and Windsurf (post_write_code JSON). All three
//! use shell-command callbacks invoking `<binary> reindex` rather than
//! the `type: "mcp_tool"` direct-MCP dispatch supported by Claude Code,
//! Cursor, and Codex.
//!
//! Each backend is idempotent (re-install replaces only GA-managed
//! entries) and supports clean uninstall (preserve unrelated entries).

use super::{HookClient, HookOutcome, VerifyOutcome};
use crate::install::json_io::{
    atomic_write_bytes, atomic_write_json, atomic_write_toml, read_json_or_empty, read_toml_or_empty,
};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Magic comment marking the GA-managed Cline hook script. Used to
/// detect re-installs + scope uninstall.
const CLINE_SCRIPT_MARKER: &str = "# graphatlas-managed: do not edit by hand";

// -------------------------------------------------------------------
// Cline — executable script at .clinerules/hooks/PostToolUse
// -------------------------------------------------------------------

/// Generate the Cline PostToolUse hook script. Earlier draft parsed
/// the stdin JSON with grep+sed to filter by tool name; that approach
/// was brittle (escape quotes broke extraction, false-positives on
/// payloads containing the literal `"tool":"write_to_file"` inside
/// other strings). Simpler + correct: reindex on every PostToolUse,
/// since `graphatlas reindex` is cheap (no-op when nothing changed)
/// and tool-name filtering is best-effort anyway.
///
/// Marker is on the SECOND line so substring scans for uninstall can
/// require header position (less spoofable than free-text scan).
fn cline_script(binary_path: &Path) -> String {
    // Shell-quote single-quotes inside the binary path. Codepath
    // `'\''` is the standard POSIX single-quote escape.
    let bin = shell_escape_sq(&binary_path.to_string_lossy());
    format!(
        "#!/usr/bin/env bash\n\
         {marker}\n\
         set -e\n\
         cat >/dev/null   # drain JSON stdin\n\
         {bin} reindex >/dev/null 2>&1 || true\n\
         printf '{{\"cancel\":false}}\\n'\n",
        marker = CLINE_SCRIPT_MARKER,
        bin = bin,
    )
}

/// POSIX-safe single-quote shell-escape: wraps the value in `'…'` and
/// replaces internal `'` with `'\''`. Defends against `binary_path`
/// containing characters that would break the bash script.
fn shell_escape_sq(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

pub(super) fn install_cline_script(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
) -> Result<HookOutcome> {
    let body = cline_script(binary_path);
    let body_bytes = body.as_bytes();
    let existed = path.exists();
    if existed {
        let current_bytes = std::fs::read(path).unwrap_or_default();
        if current_bytes == body_bytes {
            return Ok(HookOutcome::AlreadyPresent {
                client,
                path: path.to_path_buf(),
            });
        }
        let current = String::from_utf8_lossy(&current_bytes);
        // Refuse to overwrite scripts NOT authored by graphatlas. The
        // marker must appear on line 2 (header position, right after
        // the shebang) — substring-anywhere matching would be spoofable
        // by content containing the literal text.
        if !is_cline_managed_script(&current) {
            return Err(anyhow!(
                "{} exists and is not graphatlas-managed; remove it manually \
                 or rename to avoid clobbering",
                path.display()
            ));
        }
    }
    atomic_write_bytes(path, body_bytes)?;
    set_executable(path)?;
    Ok(if existed {
        HookOutcome::Added {
            client,
            path: path.to_path_buf(),
        }
    } else {
        HookOutcome::Created {
            client,
            path: path.to_path_buf(),
        }
    })
}

pub(super) fn uninstall_cline_script(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let current = std::fs::read_to_string(path).unwrap_or_default();
    if is_cline_managed_script(&current) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn verify_cline_script(path: &Path) -> Result<VerifyOutcome> {
    let body = std::fs::read_to_string(path)?;
    if !is_cline_managed_script(&body) {
        return Ok(VerifyOutcome::Missing {
            hint: format!(
                "{} exists but is not graphatlas-managed (no header marker)",
                path.display()
            ),
        });
    }
    if !body.contains(" reindex") {
        return Ok(VerifyOutcome::Malformed {
            hint: "script missing `reindex` invocation".into(),
        });
    }
    Ok(VerifyOutcome::Ok)
}

/// Strict marker check: the second line of the script must equal
/// `CLINE_SCRIPT_MARKER`. Defends against accidental matches in user
/// scripts that contain the literal marker string as part of normal
/// content (comments, log lines, etc.) — only header-position markers
/// trigger uninstall.
fn is_cline_managed_script(body: &str) -> bool {
    body.lines().nth(1).map(str::trim) == Some(CLINE_SCRIPT_MARKER)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

// -------------------------------------------------------------------
// Gemini CLI — JSON `hooks.AfterTool[]` in .gemini/settings.json
// -------------------------------------------------------------------

// Gemini matches `matcher` as a regex against the tool name. Bare `|`
// is regex alternation. (Earlier draft used `\|` — shell-egrep syntax —
// which fails silently as a literal pipe inside the tool name.)
const GEMINI_MATCHER: &str = "write_file|edit_file|run_shell_command|replace";
const HOOK_TAG_KEY: &str = "_managed_by";
const HOOK_TAG_VALUE: &str = "graphatlas";

pub(super) fn install_gemini_aftertool(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
) -> Result<HookOutcome> {
    let existed = path.exists();
    let mut doc = read_json_or_empty(path)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", path.display()))?;
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a JSON object"))?;
    let after = hooks
        .entry("AfterTool".to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.AfterTool` must be an array"))?;

    let desired = json!({
        HOOK_TAG_KEY: HOOK_TAG_VALUE,
        "matcher": GEMINI_MATCHER,
        "hooks": [{
            "type": "command",
            "command": format!("{} reindex", binary_path.to_string_lossy()),
        }],
    });
    // If an identical managed entry already exists, no rewrite — true no-op.
    if after.iter().any(|e| entries_equal(e, &desired)) {
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }
    // Otherwise drop any stale GA entry + push the new one.
    after.retain(|e| !is_ga_tagged(e));
    after.push(desired);
    atomic_write_json(path, &doc)?;
    Ok(if existed {
        HookOutcome::Added {
            client,
            path: path.to_path_buf(),
        }
    } else {
        HookOutcome::Created {
            client,
            path: path.to_path_buf(),
        }
    })
}

pub(super) fn uninstall_gemini_aftertool(path: &Path) -> Result<()> {
    let mut doc = read_json_or_empty(path)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(());
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    if let Some(after) = hooks.get_mut("AfterTool").and_then(|v| v.as_array_mut()) {
        after.retain(|e| !is_ga_tagged(e));
        if after.is_empty() {
            hooks.remove("AfterTool");
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    atomic_write_json(path, &doc)?;
    Ok(())
}

pub(super) fn verify_gemini_aftertool(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_json_or_empty(path)?;
    let after = doc
        .get("hooks")
        .and_then(|h| h.get("AfterTool"))
        .and_then(|a| a.as_array());
    let Some(arr) = after else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no hooks.AfterTool in {}", path.display()),
        });
    };
    if arr.iter().any(is_ga_tagged) {
        Ok(VerifyOutcome::Ok)
    } else {
        Ok(VerifyOutcome::Missing {
            hint: format!("AfterTool exists but contains no GA entry in {}", path.display()),
        })
    }
}

// -------------------------------------------------------------------
// Windsurf — JSON `.windsurf/hooks.json` post_write_code + post_run_command
// -------------------------------------------------------------------

const WINDSURF_HOOK_KEYS: &[&str] = &["post_write_code", "post_run_command"];

pub(super) fn install_windsurf_postwrite(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
) -> Result<HookOutcome> {
    let existed = path.exists();
    let mut doc = read_json_or_empty(path)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", path.display()))?;
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a JSON object"))?;

    let cmd = format!("{} reindex", binary_path.to_string_lossy());
    let desired_entry = json!({
        HOOK_TAG_KEY: HOOK_TAG_VALUE,
        "command": cmd.clone(),
        "powershell": cmd.clone(),
        "show_output": false,
    });

    // Probe whether every target key already contains the exact desired entry.
    let all_identical = WINDSURF_HOOK_KEYS.iter().all(|key| {
        hooks
            .get(*key)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(|e| entries_equal(e, &desired_entry)))
            .unwrap_or(false)
    });
    if all_identical {
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }

    for key in WINDSURF_HOOK_KEYS {
        let arr = hooks
            .entry((*key).to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| anyhow!("`hooks.{key}` must be an array"))?;
        arr.retain(|e| !is_ga_tagged(e));
        arr.push(desired_entry.clone());
    }
    atomic_write_json(path, &doc)?;
    Ok(if existed {
        HookOutcome::Added {
            client,
            path: path.to_path_buf(),
        }
    } else {
        HookOutcome::Created {
            client,
            path: path.to_path_buf(),
        }
    })
}

pub(super) fn uninstall_windsurf_postwrite(path: &Path) -> Result<()> {
    let mut doc = read_json_or_empty(path)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(());
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    for key in WINDSURF_HOOK_KEYS {
        if let Some(arr) = hooks.get_mut(*key).and_then(|v| v.as_array_mut()) {
            arr.retain(|e| !is_ga_tagged(e));
            if arr.is_empty() {
                hooks.remove(*key);
            }
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    atomic_write_json(path, &doc)?;
    Ok(())
}

pub(super) fn verify_windsurf_postwrite(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_json_or_empty(path)?;
    let Some(hooks) = doc.get("hooks").and_then(|v| v.as_object()) else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no hooks key in {}", path.display()),
        });
    };
    let any_ga = WINDSURF_HOOK_KEYS.iter().any(|k| {
        hooks
            .get(*k)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(is_ga_tagged))
            .unwrap_or(false)
    });
    if any_ga {
        Ok(VerifyOutcome::Ok)
    } else {
        Ok(VerifyOutcome::Missing {
            hint: format!("no GA entries in Windsurf hooks at {}", path.display()),
        })
    }
}

// -------------------------------------------------------------------
// Cursor — JSON `.cursor/hooks.json` `hooks.postToolUse[]` (camelCase!)
// -------------------------------------------------------------------
//
// Spec verified 2026-05 against cursor.com/docs/agent/hooks. Cursor
// hooks live in their own file (NOT in the MCP server config at
// .cursor/mcp.json) and the JSON key is lowercase `postToolUse`. Each
// entry has a `command` shell string. There is no `type` field, no
// `matcher` — Cursor fires postToolUse after every tool execution.

pub(super) fn install_cursor_hooks(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
) -> Result<HookOutcome> {
    let cmd = format!("{} reindex", binary_path.to_string_lossy());
    let existed = path.exists();
    let mut doc = read_json_or_empty(path)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", path.display()))?;
    // Cursor expects `version: 1` at the top.
    root.entry("version".to_string())
        .or_insert_with(|| json!(1));
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a JSON object"))?;
    let arr = hooks
        .entry("postToolUse".to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.postToolUse` must be an array"))?;
    let desired = json!({
        HOOK_TAG_KEY: HOOK_TAG_VALUE,
        "command": cmd,
    });
    let already_present = arr.iter().any(|e| entries_equal(e, &desired));
    if already_present {
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }
    let before = arr.len();
    arr.retain(|e| !is_ga_tagged(e));
    let stale_removed = before != arr.len();
    arr.push(desired);
    atomic_write_json(path, &doc)?;
    Ok(if !existed {
        HookOutcome::Created {
            client,
            path: path.to_path_buf(),
        }
    } else if stale_removed {
        // Refreshed an old GA entry — caller wants AlreadyPresent-ish.
        HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        }
    } else {
        HookOutcome::Added {
            client,
            path: path.to_path_buf(),
        }
    })
}

pub(super) fn uninstall_cursor_hooks(path: &Path) -> Result<()> {
    let mut doc = read_json_or_empty(path)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(());
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(());
    };
    if let Some(arr) = hooks.get_mut("postToolUse").and_then(|v| v.as_array_mut()) {
        arr.retain(|e| !is_ga_tagged(e));
        if arr.is_empty() {
            hooks.remove("postToolUse");
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    atomic_write_json(path, &doc)?;
    Ok(())
}

pub(super) fn verify_cursor_hooks(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_json_or_empty(path)?;
    let arr = doc
        .get("hooks")
        .and_then(|h| h.get("postToolUse"))
        .and_then(|v| v.as_array());
    let Some(arr) = arr else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no hooks.postToolUse in {}", path.display()),
        });
    };
    if arr.iter().any(is_ga_tagged) {
        Ok(VerifyOutcome::Ok)
    } else {
        Ok(VerifyOutcome::Missing {
            hint: format!("postToolUse exists but no GA entry in {}", path.display()),
        })
    }
}

// -------------------------------------------------------------------
// Codex CLI — TOML `[[hooks.PostToolUse]]` (shell command, NOT mcp_tool)
// -------------------------------------------------------------------
//
// Spec verified 2026-05: developers.openai.com/codex/hooks documents
// only `type = "command"` for the hook execution type. Earlier GA
// (v1.5 PR7) wrote `type = "mcp_tool"` which is a Claude-Code-spec
// literal that Codex silently ignores.

pub(super) fn install_codex_toml_hook(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
) -> Result<HookOutcome> {
    let cmd = format!("{} reindex", binary_path.to_string_lossy());
    let existed = path.exists();
    let mut doc = read_toml_or_empty(path)?;
    let hooks = doc
        .entry("hooks".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a TOML table"))?;
    let post = hooks
        .entry("PostToolUse".to_string())
        .or_insert_with(|| toml::Value::Array(vec![]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.PostToolUse` must be a TOML array"))?;

    let desired_entry = codex_hook_entry_toml(&cmd);
    if post.iter().any(|e| toml_entries_equal(e, &desired_entry)) {
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }
    let before = post.len();
    post.retain(|e| !is_ga_tagged_toml(e));
    let stale_removed = before != post.len();
    post.push(desired_entry);
    atomic_write_toml(path, &doc)?;
    Ok(if !existed {
        HookOutcome::Created {
            client,
            path: path.to_path_buf(),
        }
    } else if stale_removed {
        HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        }
    } else {
        HookOutcome::Added {
            client,
            path: path.to_path_buf(),
        }
    })
}

fn codex_hook_entry_toml(cmd: &str) -> toml::Value {
    let mut inner = toml::value::Table::new();
    inner.insert("type".into(), toml::Value::String("command".into()));
    inner.insert("command".into(), toml::Value::String(cmd.to_string()));
    let mut entry = toml::value::Table::new();
    entry.insert(HOOK_TAG_KEY.into(), toml::Value::String(HOOK_TAG_VALUE.into()));
    entry.insert("matcher".into(), toml::Value::String("Edit|Write|apply_patch".into()));
    entry.insert(
        "hooks".into(),
        toml::Value::Array(vec![toml::Value::Table(inner)]),
    );
    toml::Value::Table(entry)
}

pub(super) fn uninstall_codex_toml_hook(path: &Path) -> Result<()> {
    let mut doc = read_toml_or_empty(path)?;
    let Some(hooks) = doc.get_mut("hooks").and_then(|h| h.as_table_mut()) else {
        return Ok(());
    };
    if let Some(post) = hooks.get_mut("PostToolUse").and_then(|p| p.as_array_mut()) {
        post.retain(|e| !is_ga_tagged_toml(e));
        if post.is_empty() {
            hooks.remove("PostToolUse");
        }
    }
    atomic_write_toml(path, &doc)?;
    Ok(())
}

pub(super) fn verify_codex_toml_hook(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_toml_or_empty(path)?;
    let post = doc
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array());
    let Some(arr) = post else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no [[hooks.PostToolUse]] in {}", path.display()),
        });
    };
    if arr.iter().any(is_ga_tagged_toml) {
        Ok(VerifyOutcome::Ok)
    } else {
        Ok(VerifyOutcome::Missing {
            hint: format!("[[hooks.PostToolUse]] has no GA entry in {}", path.display()),
        })
    }
}

fn is_ga_tagged_toml(entry: &toml::Value) -> bool {
    entry
        .get(HOOK_TAG_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s == HOOK_TAG_VALUE)
        .unwrap_or(false)
}

fn toml_entries_equal(a: &toml::Value, b: &toml::Value) -> bool {
    a == b
}

fn entries_equal(a: &Value, b: &Value) -> bool {
    a == b
}

// -------------------------------------------------------------------
// Shared helpers
// -------------------------------------------------------------------

fn is_ga_tagged(entry: &Value) -> bool {
    entry
        .get(HOOK_TAG_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s == HOOK_TAG_VALUE)
        .unwrap_or(false)
}

#[allow(dead_code)]
fn target_path(_p: &Path) -> PathBuf {
    unreachable!()
}
