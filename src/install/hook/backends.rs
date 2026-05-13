//! v1.5 PR7 — Hook installer backend implementations.
//!
//! Split out from `hook/mod.rs` to keep both files under the 350 LOC
//! threshold. The public API (`install_hook`, `verify_hook`, etc.) lives
//! in `mod.rs`; this file holds the JSON + TOML format-specific writers
//! and the shared atomic-write / symlink-defense helpers.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

use super::{
    HookClient, HookOutcome, VerifyOutcome, HOOK_MATCHER, HOOK_SERVER, HOOK_TOOL, HOOK_TYPE,
};

// --------------------------------------------------------------------
// JSON hook (claude-code, cursor) — shared shape
// --------------------------------------------------------------------

pub(super) fn install_json_hook(client: HookClient, path: &Path) -> Result<HookOutcome> {
    let mut doc = read_json_or_empty(path)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", path.display()))?;

    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}));
    let hooks_map = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a JSON object"))?;
    let post = hooks_map
        .entry("PostToolUse".to_string())
        .or_insert_with(|| json!([]));
    let post_arr = post
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.PostToolUse` must be an array"))?;

    if post_arr.iter().any(is_ga_hook_entry) {
        // AS-002 — idempotent. File untouched.
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }

    let had_file = path.exists();
    post_arr.push(ga_hook_entry_json());
    atomic_write_json(path, &doc, false)?;

    Ok(if had_file {
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

pub(super) fn uninstall_json_hook(path: &Path) -> Result<()> {
    let mut doc = read_json_or_empty(path)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(());
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return Ok(());
    };
    if let Some(post) = hooks.get_mut("PostToolUse").and_then(|p| p.as_array_mut()) {
        post.retain(|e| !is_ga_hook_entry(e));
        if post.is_empty() {
            hooks.remove("PostToolUse");
        }
    }
    atomic_write_json(path, &doc, false)
}

pub(super) fn verify_json_hook(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_json_or_empty(path)?;
    let Some(post) = doc
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array())
    else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no hooks.PostToolUse entry in {}", path.display()),
        });
    };
    // Look for a GA entry. If we find one with the right matcher + tool,
    // VerifyOutcome::Ok. If we find an entry that references GA but the
    // matcher or tool is wrong, that's Malformed.
    for entry in post {
        let refs_ga = entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|arr| {
                arr.iter().any(|h| {
                    h.get("server").and_then(|s| s.as_str()) == Some(HOOK_SERVER)
                        && h.get("tool").and_then(|s| s.as_str()) == Some(HOOK_TOOL)
                })
            })
            .unwrap_or(false);
        if !refs_ga {
            continue;
        }
        let matcher_ok = entry.get("matcher").and_then(|m| m.as_str()) == Some(HOOK_MATCHER);
        if matcher_ok {
            return Ok(VerifyOutcome::Ok);
        }
        return Ok(VerifyOutcome::Malformed {
            hint: format!(
                "found GA entry but matcher != `{}` (got {:?})",
                HOOK_MATCHER,
                entry.get("matcher").and_then(|m| m.as_str())
            ),
        });
    }
    Ok(VerifyOutcome::Missing {
        hint: format!("PostToolUse exists but contains no GA entry in {}", path.display()),
    })
}

fn is_ga_hook_entry(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter().any(|h| {
                h.get("server").and_then(|s| s.as_str()) == Some(HOOK_SERVER)
                    && h.get("tool").and_then(|s| s.as_str()) == Some(HOOK_TOOL)
            })
        })
        .unwrap_or(false)
}

fn ga_hook_entry_json() -> Value {
    json!({
        "matcher": HOOK_MATCHER,
        "hooks": [{
            "type": HOOK_TYPE,
            "server": HOOK_SERVER,
            "tool": HOOK_TOOL,
        }],
    })
}

// --------------------------------------------------------------------
// TOML hook (codex) — same logical shape, different format
// --------------------------------------------------------------------

pub(super) fn install_toml_hook(client: HookClient, path: &Path) -> Result<HookOutcome> {
    let mut doc = read_toml_or_empty(path)?;
    let hooks = doc
        .entry("hooks".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    let hooks_tbl = hooks
        .as_table_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a TOML table"))?;
    let post = hooks_tbl
        .entry("PostToolUse".to_string())
        .or_insert_with(|| toml::Value::Array(vec![]));
    let post_arr = post
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.PostToolUse` must be a TOML array"))?;

    if post_arr.iter().any(is_ga_hook_entry_toml) {
        return Ok(HookOutcome::AlreadyPresent {
            client,
            path: path.to_path_buf(),
        });
    }

    let had_file = path.exists();
    post_arr.push(ga_hook_entry_toml());
    atomic_write_toml(path, &doc)?;
    Ok(if had_file {
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

pub(super) fn uninstall_toml_hook(path: &Path) -> Result<()> {
    let mut doc = read_toml_or_empty(path)?;
    let Some(hooks) = doc.get_mut("hooks").and_then(|h| h.as_table_mut()) else {
        return Ok(());
    };
    if let Some(post) = hooks.get_mut("PostToolUse").and_then(|p| p.as_array_mut()) {
        post.retain(|e| !is_ga_hook_entry_toml(e));
        if post.is_empty() {
            hooks.remove("PostToolUse");
        }
    }
    atomic_write_toml(path, &doc)
}

pub(super) fn verify_toml_hook(path: &Path) -> Result<VerifyOutcome> {
    let doc = read_toml_or_empty(path)?;
    let Some(post) = doc
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array())
    else {
        return Ok(VerifyOutcome::Missing {
            hint: format!("no [hooks.PostToolUse] in {}", path.display()),
        });
    };
    for entry in post {
        if !is_ga_hook_entry_toml(entry) {
            continue;
        }
        let matcher_ok = entry.get("matcher").and_then(|m| m.as_str()) == Some(HOOK_MATCHER);
        if matcher_ok {
            return Ok(VerifyOutcome::Ok);
        }
        return Ok(VerifyOutcome::Malformed {
            hint: "found GA TOML entry but matcher mismatch".to_string(),
        });
    }
    Ok(VerifyOutcome::Missing {
        hint: format!("hooks.PostToolUse exists but no GA entry in {}", path.display()),
    })
}

fn is_ga_hook_entry_toml(entry: &toml::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter().any(|h| {
                h.get("server").and_then(|s| s.as_str()) == Some(HOOK_SERVER)
                    && h.get("tool").and_then(|s| s.as_str()) == Some(HOOK_TOOL)
            })
        })
        .unwrap_or(false)
}

fn ga_hook_entry_toml() -> toml::Value {
    let mut inner = toml::value::Table::new();
    inner.insert("type".into(), HOOK_TYPE.into());
    inner.insert("server".into(), HOOK_SERVER.into());
    inner.insert("tool".into(), HOOK_TOOL.into());
    let mut entry = toml::value::Table::new();
    entry.insert("matcher".into(), HOOK_MATCHER.into());
    entry.insert("hooks".into(), toml::Value::Array(vec![toml::Value::Table(inner)]));
    toml::Value::Table(entry)
}

// --------------------------------------------------------------------
// Shared helpers — symlink defense, atomic write, parent creation
// --------------------------------------------------------------------

pub(super) fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
    }
    Ok(())
}

/// AS-001b — refuse to follow symlinks at the target path unless the
/// caller opts in. Defends against attacker-controlled symlinks pointing
/// the installer at sensitive files (e.g. `/etc/passwd`).
pub(super) fn refuse_symlink_unless(path: &Path, follow_symlinks: bool) -> Result<()> {
    if follow_symlinks {
        return Ok(());
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow::Error::from(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "Refusing to follow symlink at {}; use --follow-symlinks to override.",
            path.display()
        ));
    }
    Ok(())
}

fn read_json_or_empty(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} is corrupt JSON; refusing to overwrite (fix or remove the file)",
            path.display()
        )
    })
}

fn read_toml_or_empty(path: &Path) -> Result<toml::value::Table> {
    if !path.exists() {
        return Ok(toml::value::Table::new());
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(toml::value::Table::new());
    }
    let s = std::str::from_utf8(&bytes).context("toml config not utf-8")?;
    toml::from_str(s).with_context(|| {
        format!(
            "{} is corrupt TOML; refusing to overwrite (fix or remove the file)",
            path.display()
        )
    })
}

/// AS-001 — atomic write with O_EXCL sibling tempfile, fsync, rename.
/// `_follow_symlinks` is reserved for callers that already passed the
/// symlink check upstream and intentionally want the rename to follow.
fn atomic_write_json(path: &Path, value: &Value, _follow_symlinks: bool) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(value).context("serialize hook config")?;
    atomic_write_bytes(path, &serialized)
}

fn atomic_write_toml(path: &Path, value: &toml::value::Table) -> Result<()> {
    let serialized = toml::to_string_pretty(value).context("serialize hook TOML")?;
    atomic_write_bytes(path, serialized.as_bytes())
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    // tempfile::Builder + O_EXCL semantics: a random 8-char suffix
    // guarantees no collision; persist() renames atomically on POSIX.
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp.")
        .suffix(".hook")
        .rand_bytes(8)
        .tempfile_in(parent)
        .with_context(|| format!("create tempfile in {}", parent.display()))?;
    use std::io::Write;
    tmp.write_all(bytes).context("write tempfile")?;
    tmp.as_file_mut().sync_all().context("fsync tempfile")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o600));
    }
    tmp.persist(path)
        .map_err(|e| anyhow!("rename tempfile -> {}: {}", path.display(), e))?;
    Ok(())
}
