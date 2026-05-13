//! `ga init` integration tests — exercise the installer subsystem end
//! to end against a temp project root. Coverage:
//!
//! - Fresh project: skill + CLAUDE.md + permissions are written.
//! - Idempotent re-run: second invocation is a no-op (no duplicate
//!   block, no duplicate allow-list entry).
//! - Pre-existing CLAUDE.md is preserved (managed block appended).
//! - Pre-existing settings.json keys are preserved (deep merge).
//! - `--with-hook` inserts the SessionStart entry; `--remove-hook`
//!   removes only the managed entry.
//! - Unrelated hook entries (e.g. v1.5 PR7 PostToolUse reindex) are
//!   preserved across install + uninstall.

use graphatlas::cmd_init::{cmd_init, InitOptions};
use graphatlas::install::claudemd::{BEGIN_MARKER, END_MARKER};
use graphatlas::install::permissions::ALLOWED_PREFIX;
use graphatlas::install::session_hook::{MANAGED_TAG_KEY, MANAGED_TAG_VALUE, SESSION_START_KEY};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("scratch dir")
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn default_opts(root: &Path) -> InitOptions {
    InitOptions {
        project_root: Some(root.to_path_buf()),
        binary_path: Some(PathBuf::from("/usr/local/bin/graphatlas")),
        ..InitOptions::default()
    }
}

#[test]
fn fresh_project_writes_all_three_artifacts() {
    let scratch = scratch();
    let root = scratch.path();

    cmd_init(default_opts(root)).expect("init");

    assert!(root.join(".claude/skills/graphatlas.md").exists());
    let claudemd = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert!(claudemd.contains(BEGIN_MARKER));
    assert!(claudemd.contains(END_MARKER));

    let settings = read_json(&root.join(".claude/settings.json"));
    let allow = &settings["permissions"]["allow"];
    let allow_arr = allow.as_array().expect("allow array");
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some(&format!("{ALLOWED_PREFIX}*"))),
        "expected wildcard in allow: {allow:?}"
    );
    // Hook should NOT be present by default.
    assert!(settings.get("hooks").is_none());
}

#[test]
fn idempotent_rerun_no_duplicates() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(default_opts(root)).expect("first init");
    cmd_init(default_opts(root)).expect("second init");

    let claudemd = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert_eq!(
        claudemd.matches(BEGIN_MARKER).count(),
        1,
        "duplicate GA block in CLAUDE.md after re-run"
    );

    let settings = read_json(&root.join(".claude/settings.json"));
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    let count = allow
        .iter()
        .filter(|v| v.as_str() == Some(&format!("{ALLOWED_PREFIX}*")))
        .count();
    assert_eq!(count, 1, "duplicate allow-list entry");
}

#[test]
fn preserves_existing_claudemd_content() {
    let scratch = scratch();
    let root = scratch.path();
    let existing = "# My Project\n\nSome notes the user already wrote.\n";
    fs::write(root.join("CLAUDE.md"), existing).unwrap();

    cmd_init(default_opts(root)).expect("init");

    let after = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert!(after.contains("# My Project"), "lost user heading");
    assert!(
        after.contains("Some notes the user already wrote."),
        "lost user notes"
    );
    assert!(after.contains(BEGIN_MARKER));
    assert!(after.contains(END_MARKER));
}

#[test]
fn preserves_existing_settings_keys() {
    let scratch = scratch();
    let root = scratch.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    let pre = json!({
        "hooks": {
            "PostToolUse": [
                {"matcher": "Edit", "hooks": [{"type": "mcp_tool", "server": "other"}]}
            ]
        },
        "custom_user_key": "do not touch"
    });
    fs::write(
        root.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&pre).unwrap(),
    )
    .unwrap();

    cmd_init(default_opts(root)).expect("init");

    let after = read_json(&root.join(".claude/settings.json"));
    assert_eq!(after["custom_user_key"], json!("do not touch"));
    assert_eq!(after["hooks"]["PostToolUse"][0]["matcher"], json!("Edit"));
    assert!(after["permissions"]["allow"].is_array());
}

#[test]
fn with_hook_inserts_session_start_entry() {
    let scratch = scratch();
    let root = scratch.path();
    let mut opts = default_opts(root);
    opts.with_hook = true;
    cmd_init(opts).expect("init --with-hook");

    let settings = read_json(&root.join(".claude/settings.json"));
    let arr = settings["hooks"][SESSION_START_KEY]
        .as_array()
        .expect("SessionStart array");
    let managed: Vec<&Value> = arr
        .iter()
        .filter(|e| e[MANAGED_TAG_KEY].as_str() == Some(MANAGED_TAG_VALUE))
        .collect();
    assert_eq!(managed.len(), 1, "exactly one managed entry");
    let cmd = managed[0]["hooks"][0]["command"]
        .as_str()
        .expect("command string");
    assert!(cmd.ends_with("hook session-start"), "command was {cmd:?}");
}

#[test]
fn remove_hook_preserves_unrelated_entries() {
    let scratch = scratch();
    let root = scratch.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    // Pre-seed an unrelated SessionStart entry the user owns.
    let pre = json!({
        "hooks": {
            "SessionStart": [
                {"matcher": "*", "hooks": [{"type": "command", "command": "echo hi"}]}
            ]
        }
    });
    fs::write(
        root.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&pre).unwrap(),
    )
    .unwrap();

    // Install with hook, then remove.
    let mut opts = default_opts(root);
    opts.with_hook = true;
    cmd_init(opts).expect("install hook");

    let mut opts2 = default_opts(root);
    opts2.remove_hook = true;
    cmd_init(opts2).expect("remove hook");

    let after = read_json(&root.join(".claude/settings.json"));
    let arr = after["hooks"][SESSION_START_KEY]
        .as_array()
        .expect("SessionStart array still exists");
    assert_eq!(arr.len(), 1, "user's entry preserved");
    let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
    assert_eq!(cmd, "echo hi");
    // No GA-managed entry left.
    assert!(arr
        .iter()
        .all(|e| e[MANAGED_TAG_KEY].as_str() != Some(MANAGED_TAG_VALUE)));
}

#[test]
fn with_hook_replaces_stale_managed_entry() {
    let scratch = scratch();
    let root = scratch.path();
    let mut opts = default_opts(root);
    opts.with_hook = true;
    opts.binary_path = Some(PathBuf::from("/old/path/graphatlas"));
    cmd_init(opts).expect("first install with stale path");

    let mut opts2 = default_opts(root);
    opts2.with_hook = true;
    opts2.binary_path = Some(PathBuf::from("/new/path/graphatlas"));
    cmd_init(opts2).expect("second install with new path");

    let settings = read_json(&root.join(".claude/settings.json"));
    let arr = settings["hooks"][SESSION_START_KEY].as_array().unwrap();
    let managed: Vec<&Value> = arr
        .iter()
        .filter(|e| e[MANAGED_TAG_KEY].as_str() == Some(MANAGED_TAG_VALUE))
        .collect();
    assert_eq!(managed.len(), 1, "no duplicate managed entry after refresh");
    let cmd = managed[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.starts_with("/new/path/graphatlas"), "stale path lingered: {cmd}");
}

#[test]
fn interactive_prompt_pick_skill_only() {
    let scratch = scratch();
    let root = scratch.path();
    // Answer: skill=y, claudemd=n, permissions=n, hook=n, confirm=y.
    let answers = b"y\nn\nn\nn\ny\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let result = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow")
        .expect("user confirmed");
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("graphatlas init — interactive setup"));
    assert!(rendered.contains("Skill file"));
    assert!(rendered.contains("SessionStart hook"));
    // We can't access ResolvedFlags fields (private), but we can verify
    // applying the result writes only the skill file.
    assert!(rendered.contains("Summary:"));
    // Re-derive by running cmd_init with the flags we picked: skill only.
    // For now just assert the rendered summary matches our intent.
    assert!(rendered.contains("✓ skill"));
    assert!(rendered.contains("✗ CLAUDE.md"));
    assert!(rendered.contains("✗ permissions"));
    assert!(rendered.contains("✗ SessionStart hook"));
    let _ = result;
}

#[test]
fn interactive_default_yes_on_empty_input_for_first_three() {
    let scratch = scratch();
    let root = scratch.path();
    // Pressing Enter on each prompt: defaults are y/y/y/n for the four
    // components, plus y for confirm.
    let answers = b"\n\n\n\n\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let _ = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow")
        .expect("user confirmed");
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("✓ skill"));
    assert!(rendered.contains("✓ CLAUDE.md"));
    assert!(rendered.contains("✓ permissions"));
    assert!(rendered.contains("✗ SessionStart hook"));
}

#[test]
fn interactive_decline_at_confirm_aborts() {
    let scratch = scratch();
    let root = scratch.path();
    // All y, then 'n' at the confirm.
    let answers = b"y\ny\ny\ny\nn\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let outcome = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow");
    assert!(outcome.is_none(), "user said n at confirm → no flags");
}

#[test]
fn interactive_none_selected_aborts() {
    let scratch = scratch();
    let root = scratch.path();
    let answers = b"n\nn\nn\nn\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let outcome = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow");
    assert!(outcome.is_none());
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("Nothing selected"));
}

#[test]
fn interactive_preflight_reports_existing_state() {
    let scratch = scratch();
    let root = scratch.path();
    // Seed: skill + CLAUDE.md + settings.json pre-existing.
    let mut opts = default_opts(root);
    opts.yes = true;
    cmd_init(opts).expect("seed install");

    let answers = b"\n\n\n\n\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let _ = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer);
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("will refresh"), "preflight: {rendered}");
    assert!(rendered.contains("GA block present"));
    assert!(rendered.contains("settings.json exists"));
}

#[test]
fn interactive_reprompts_on_invalid_input() {
    let scratch = scratch();
    let root = scratch.path();
    // First answer is junk → tool should re-prompt and accept 'y' on retry.
    // After that, defaults through the remaining prompts + confirm.
    let answers = b"xyz\ny\n\n\n\n\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let outcome = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow")
        .expect("eventually confirmed");
    let rendered = String::from_utf8_lossy(&writer);
    assert!(
        rendered.contains("please answer y or n"),
        "missing reprompt hint: {rendered}"
    );
    assert!(outcome.skill, "user answered y on retry");
}

#[test]
fn interactive_eof_falls_back_to_defaults() {
    let scratch = scratch();
    let root = scratch.path();
    // Empty stdin — every read_line returns 0. Each prompt should adopt
    // its declared default rather than spinning forever.
    let mut reader = std::io::Cursor::new(&b""[..]);
    let mut writer: Vec<u8> = Vec::new();
    let outcome = graphatlas::cmd_init::interactive_pick_io(root, &mut reader, &mut writer)
        .expect("interactive flow")
        .expect("EOF defaults to proceed");
    // Defaults: skill/claudemd/permissions = true, hook = false.
    assert!(outcome.skill);
    assert!(outcome.claudemd);
    assert!(outcome.permissions);
    assert!(!outcome.hook);
}

#[test]
fn hook_session_start_subcommand_prints_reminder() {
    // Calling the in-process function should write reminder text. We
    // can't capture stdout from print! easily without redirection, so
    // assert the underlying constant has the expected anchor strings.
    let body = graphatlas::cmd_hook::SESSION_START_REMINDER;
    assert!(body.contains("GraphAtlas code-graph available"));
    assert!(body.contains("ga_callers"));
    assert!(body.contains("ga_impact"));
}
