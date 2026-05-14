//! v1.5 PR7 — hook installer tests (triggers sub-spec S-001 + S-002).
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-triggers.md`
//! S-001 AS-001/001b/002/003/004 (Claude Code, P0) + S-002 AS-005/006
//! (Cursor + Codex, P1).

use graphatlas::install::{
    install_hook, install_hook_at, uninstall_hook, verify_hook, HookClient, HookOutcome,
    VerifyOutcome,
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

// =====================================================================
// S-001 AS-001 — fresh install writes the spec-literal JSON
// =====================================================================

#[test]
fn as_001_fresh_install_creates_claude_code_settings_with_postrooluse_entry() {
    let tmp = TempDir::new().unwrap();
    let outcome = install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let path = match outcome {
        HookOutcome::Created { path, .. } => path,
        other => panic!("AS-001: fresh install must return Created, got {other:?}"),
    };
    assert_eq!(path, tmp.path().join(".claude").join("settings.json"));

    let v = read_json(&path);
    let post = v["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1, "AS-001: exactly one PostToolUse entry");
    assert_eq!(post[0]["matcher"], "Edit|Write|Bash");
    // Spec drift fix — Claude Code 2026-05 hooks require type "mcp_tool"
    // (not "mcp"); see https://code.claude.com/docs/en/hooks. Prior code
    // emitted "mcp" which made Claude Code reject the entire settings.json
    // with "Settings Error · type: Invalid input".
    assert_eq!(post[0]["hooks"][0]["type"], "mcp_tool");
    assert_eq!(post[0]["hooks"][0]["server"], "graphatlas");
    assert_eq!(post[0]["hooks"][0]["tool"], "ga_reindex");
}

#[test]
fn as_001_install_preserves_existing_unrelated_keys() {
    let tmp = TempDir::new().unwrap();
    let settings = tmp.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let pre = json!({
        "permissions": { "allow": ["Read", "Bash(git status:*)"] },
        "model": "sonnet",
        "hooks": {
            "Stop": [{"matcher": "*", "hooks": [{"command": "echo done"}]}]
        }
    });
    fs::write(&settings, serde_json::to_vec_pretty(&pre).unwrap()).unwrap();

    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let v = read_json(&settings);
    assert_eq!(v["permissions"]["allow"][0], "Read");
    assert_eq!(v["model"], "sonnet");
    assert!(
        v["hooks"]["Stop"].is_array(),
        "AS-001: unrelated hook arrays preserved"
    );
    assert_eq!(
        v["hooks"]["PostToolUse"][0]["hooks"][0]["tool"],
        "ga_reindex"
    );
}

#[test]
fn as_001_atomic_write_leaves_file_at_target_path_only() {
    // After install, only `settings.json` (+ advisory `.ga-lock`
    // sidecar from the TOCTOU defense) should exist — no leftover
    // `.tmp.*` siblings (proving the rename completed atomically).
    let tmp = TempDir::new().unwrap();
    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let mut entries: Vec<_> = fs::read_dir(tmp.path().join(".claude"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".ga-lock"))
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["settings.json".to_string()]);
}

// =====================================================================
// S-001 AS-001b — symlink target refused
// =====================================================================

#[cfg(unix)]
#[test]
fn as_001b_install_refuses_symlink_target_without_follow_flag() {
    let tmp = TempDir::new().unwrap();
    let sensitive = tmp.path().join("sensitive.txt");
    fs::write(&sensitive, "do not touch").unwrap();
    let claude_dir = tmp.path().join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    std::os::unix::fs::symlink(&sensitive, &settings).unwrap();

    let err = install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Refusing to follow symlink"),
        "AS-001b: refusal message must mention symlink; got {msg:?}"
    );
    assert_eq!(
        fs::read_to_string(&sensitive).unwrap(),
        "do not touch",
        "AS-001b: sensitive target must remain untouched"
    );
}

#[cfg(unix)]
#[test]
fn as_001b_follow_symlinks_flag_allows_install() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real-settings.json");
    fs::write(&real, "{}").unwrap();
    let claude_dir = tmp.path().join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    std::os::unix::fs::symlink(&real, &settings).unwrap();
    // With explicit override, install proceeds (the atomic rename may
    // replace the symlink; we just assert it didn't error).
    install_hook(HookClient::ClaudeCode, tmp.path(), true).unwrap();
}

// =====================================================================
// S-001 AS-002 — re-install idempotent
// =====================================================================

#[test]
fn as_002_reinstall_is_idempotent_no_duplicate_entry() {
    let tmp = TempDir::new().unwrap();
    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let outcome = install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    assert!(
        matches!(outcome, HookOutcome::AlreadyPresent { .. }),
        "AS-002: re-install must report AlreadyPresent; got {outcome:?}"
    );
    let v = read_json(&tmp.path().join(".claude").join("settings.json"));
    assert_eq!(
        v["hooks"]["PostToolUse"].as_array().unwrap().len(),
        1,
        "AS-002: re-install must not duplicate the entry"
    );
}

// =====================================================================
// S-001 AS-003 — verify mode
// =====================================================================

#[test]
fn as_003_verify_returns_missing_when_file_absent() {
    let tmp = TempDir::new().unwrap();
    let outcome = verify_hook(HookClient::ClaudeCode, tmp.path()).unwrap();
    assert!(
        matches!(outcome, VerifyOutcome::Missing { .. }),
        "AS-003: absent file → Missing; got {outcome:?}"
    );
}

#[test]
fn as_003_verify_returns_ok_after_install() {
    let tmp = TempDir::new().unwrap();
    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let outcome = verify_hook(HookClient::ClaudeCode, tmp.path()).unwrap();
    assert_eq!(
        outcome,
        VerifyOutcome::Ok,
        "AS-003: installed hook must verify Ok"
    );
}

#[test]
fn as_003_verify_returns_malformed_when_matcher_wrong() {
    let tmp = TempDir::new().unwrap();
    let settings = tmp.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let pre = json!({
        "hooks": {
            "PostToolUse": [{
                "matcher": "Read",
                "hooks": [{"type": "mcp_tool", "server": "graphatlas", "tool": "ga_reindex"}]
            }]
        }
    });
    fs::write(&settings, serde_json::to_vec_pretty(&pre).unwrap()).unwrap();
    let outcome = verify_hook(HookClient::ClaudeCode, tmp.path()).unwrap();
    assert!(
        matches!(outcome, VerifyOutcome::Malformed { .. }),
        "AS-003: wrong matcher → Malformed; got {outcome:?}"
    );
}

// =====================================================================
// S-001 AS-004 — uninstall
// =====================================================================

#[test]
fn as_004_uninstall_removes_only_ga_entry_preserves_others() {
    let tmp = TempDir::new().unwrap();
    let settings = tmp.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    // Pre-existing config with GA + another unrelated PostToolUse hook.
    let pre = json!({
        "hooks": {
            "PostToolUse": [
                {"matcher": "Bash", "hooks": [{"command": "echo bash done"}]},
            ]
        }
    });
    fs::write(&settings, serde_json::to_vec_pretty(&pre).unwrap()).unwrap();
    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    // Now uninstall.
    uninstall_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let v = read_json(&settings);
    let post = v["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1, "AS-004: only the unrelated entry remains");
    assert_eq!(post[0]["matcher"], "Bash");
    assert_eq!(post[0]["hooks"][0]["command"], "echo bash done");
}

#[test]
fn as_004_uninstall_prunes_empty_posttooluse_array() {
    let tmp = TempDir::new().unwrap();
    install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    uninstall_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
    let v = read_json(&tmp.path().join(".claude").join("settings.json"));
    assert!(
        v["hooks"].get("PostToolUse").is_none(),
        "AS-004: empty PostToolUse array must be pruned; got {v}"
    );
}

#[test]
fn as_004_uninstall_no_op_when_file_absent() {
    let tmp = TempDir::new().unwrap();
    // Must not error even if config never existed.
    uninstall_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap();
}

// =====================================================================
// S-001 — corrupt-JSON guard (error path edge case)
// =====================================================================

#[test]
fn install_refuses_corrupt_json_does_not_overwrite() {
    let tmp = TempDir::new().unwrap();
    let settings = tmp.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, b"{not valid json").unwrap();
    let err = install_hook(HookClient::ClaudeCode, tmp.path(), false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("corrupt JSON") || msg.contains("expected"),
        "corrupt JSON must surface a clear error; got {msg:?}"
    );
    // Original bytes preserved.
    assert_eq!(fs::read(&settings).unwrap(), b"{not valid json");
}

// =====================================================================
// S-002 AS-005 — Cursor template
// =====================================================================

#[test]
fn as_005_install_for_cursor_writes_cursor_hooks_json() {
    // Spec verified 2026-05: Cursor hooks live at `.cursor/hooks.json`
    // (NOT `.cursor/mcp.json` — that's the MCP server registry). Key
    // is camelCase `postToolUse` with shell `command`. Earlier shape
    // (`PostToolUse` + `type: "mcp_tool"`) was silently ignored by Cursor.
    let tmp = TempDir::new().unwrap();
    let outcome = install_hook(HookClient::Cursor, tmp.path(), false).unwrap();
    let path = match outcome {
        HookOutcome::Created { path, .. } => path,
        other => panic!("AS-005: Cursor fresh install must return Created, got {other:?}"),
    };
    assert_eq!(path, tmp.path().join(".cursor").join("hooks.json"));
    let v = read_json(&path);
    assert_eq!(v["version"], 1);
    let post = v["hooks"]["postToolUse"].as_array().unwrap();
    let ga = post
        .iter()
        .find(|e| e["_managed_by"] == "graphatlas")
        .expect("GA-managed entry");
    let cmd = ga["command"].as_str().unwrap();
    assert!(cmd.ends_with(" reindex"));
}

#[test]
fn as_005_cursor_verify_after_install_is_ok() {
    let tmp = TempDir::new().unwrap();
    install_hook(HookClient::Cursor, tmp.path(), false).unwrap();
    assert_eq!(
        verify_hook(HookClient::Cursor, tmp.path()).unwrap(),
        VerifyOutcome::Ok
    );
}

// =====================================================================
// S-002 AS-006 — Codex TOML
// =====================================================================

#[test]
fn as_006_install_for_codex_writes_toml_with_postrooluse_entry() {
    // Spec verified 2026-05 against developers.openai.com/codex/hooks:
    // Codex hooks use `type = "command"` (shell), NOT `mcp_tool`. The
    // hook command invokes `<bin> reindex` rather than calling the MCP
    // server directly. Matcher uses Codex's own tool names (Edit,
    // Write, apply_patch).
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join(".codex").join("config.toml");
    let outcome = install_hook_at(HookClient::Codex, &cfg, false).unwrap();
    assert!(
        matches!(outcome, HookOutcome::Created { .. }),
        "AS-006: Codex fresh install must return Created; got {outcome:?}"
    );
    let body = fs::read_to_string(&cfg).unwrap();
    assert!(
        body.contains("PostToolUse"),
        "TOML must include PostToolUse"
    );
    assert!(
        body.contains("type = \"command\""),
        "Codex hooks use type=\"command\", not mcp_tool: {body}"
    );
    assert!(body.contains("reindex"), "command must invoke reindex");
    assert!(
        body.contains("apply_patch"),
        "matcher must include apply_patch"
    );
}
