//! `ga init` integration tests. Coverage:
//!
//! - Claude Code single-platform install (skill + CLAUDE.md + permissions).
//! - Idempotent re-run.
//! - Pre-existing CLAUDE.md / settings.json content preserved on merge.
//! - `--with-hook` SessionStart entry; `--remove-hook` preserves unrelated entries.
//! - Multi-platform install: Cursor, Cline, Codex, Gemini per-platform smoke
//!   (file shape + idempotency).
//! - Interactive numbered-toggle picker: defaults, toggle parser, all/none/done,
//!   abort, EOF fallback.

use graphatlas::cmd_init::{cmd_init, InitOptions};
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// RAII guard that swaps HOME for the test body and restores it on
/// Drop — even if the test panics. Also serializes via a global Mutex
/// because HOME is process-global state.
struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    prev: Option<OsString>,
}

impl HomeGuard {
    fn set(home: &std::path::Path) -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        HomeGuard { _lock: lock, prev }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
use graphatlas::install::claudemd::{BEGIN_MARKER, END_MARKER};
use graphatlas::install::permissions::ALLOWED_PREFIX;
use graphatlas::install::platforms::Platform;
use graphatlas::install::session_hook::{MANAGED_TAG_KEY, MANAGED_TAG_VALUE, SESSION_START_KEY};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("scratch dir")
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn read_toml(path: &Path) -> toml::Value {
    let s = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    toml::from_str(&s).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Default options targeting Claude Code only — preserves the v1 test
/// semantics (skill + CLAUDE.md + permissions written into scratch).
fn claude_opts(root: &Path) -> InitOptions {
    InitOptions {
        project_root: Some(root.to_path_buf()),
        platforms: vec![Platform::ClaudeCode],
        yes: true,
        binary_path: Some(PathBuf::from("/usr/local/bin/graphatlas")),
        ..InitOptions::default()
    }
}

// ===================================================================
// Claude Code single-platform smoke (preserved from v1)
// ===================================================================

#[test]
fn fresh_project_writes_all_three_claude_artifacts() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(claude_opts(root)).expect("init");

    assert!(root.join(".claude/skills/graphatlas.md").exists());
    let claudemd = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert!(claudemd.contains(BEGIN_MARKER));
    assert!(claudemd.contains(END_MARKER));

    let settings = read_json(&root.join(".claude/settings.json"));
    let allow = settings["permissions"]["allow"]
        .as_array()
        .expect("allow array");
    assert!(allow
        .iter()
        .any(|v| v.as_str() == Some(&format!("{ALLOWED_PREFIX}*"))));
    // Reindex hook IS installed by default (per Phase 2.5 integration);
    // SessionStart hook is NOT.
    let hooks = settings.get("hooks").expect("hooks present after init");
    assert!(hooks.get("PostToolUse").is_some(), "reindex hook expected");
    assert!(hooks.get("SessionStart").is_none(), "SessionStart opt-in");
}

#[test]
fn idempotent_rerun_no_duplicates_claude() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(claude_opts(root)).expect("first init");
    cmd_init(claude_opts(root)).expect("second init");

    let claudemd = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert_eq!(claudemd.matches(BEGIN_MARKER).count(), 1);

    let settings = read_json(&root.join(".claude/settings.json"));
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    assert_eq!(
        allow
            .iter()
            .filter(|v| v.as_str() == Some(&format!("{ALLOWED_PREFIX}*")))
            .count(),
        1
    );
}

#[test]
fn preserves_existing_claudemd_content() {
    let scratch = scratch();
    let root = scratch.path();
    fs::write(root.join("CLAUDE.md"), "# My Project\n\nUser notes.\n").unwrap();

    cmd_init(claude_opts(root)).expect("init");

    let after = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert!(after.contains("# My Project"));
    assert!(after.contains("User notes."));
    assert!(after.contains(BEGIN_MARKER));
}

#[test]
fn preserves_existing_settings_keys() {
    let scratch = scratch();
    let root = scratch.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    let pre = json!({
        "hooks": {"PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "mcp_tool", "server": "other"}]}]},
        "custom_user_key": "do not touch"
    });
    fs::write(
        root.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&pre).unwrap(),
    )
    .unwrap();

    cmd_init(claude_opts(root)).expect("init");

    let after = read_json(&root.join(".claude/settings.json"));
    assert_eq!(after["custom_user_key"], json!("do not touch"));
    assert_eq!(after["hooks"]["PostToolUse"][0]["matcher"], json!("Edit"));
    assert!(after["permissions"]["allow"].is_array());
}

#[test]
fn with_hook_inserts_session_start_entry() {
    let scratch = scratch();
    let root = scratch.path();
    let mut opts = claude_opts(root);
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
    assert_eq!(managed.len(), 1);
    let cmd = managed[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.ends_with("hook session-start"));
}

#[test]
fn remove_hook_preserves_unrelated_entries() {
    let scratch = scratch();
    let root = scratch.path();
    fs::create_dir_all(root.join(".claude")).unwrap();
    let pre = json!({
        "hooks": {"SessionStart": [{"matcher": "*", "hooks": [{"type": "command", "command": "echo hi"}]}]}
    });
    fs::write(
        root.join(".claude/settings.json"),
        serde_json::to_vec_pretty(&pre).unwrap(),
    )
    .unwrap();

    let mut opts = claude_opts(root);
    opts.with_hook = true;
    cmd_init(opts).expect("install hook");

    let mut opts2 = claude_opts(root);
    opts2.remove_hook = true;
    cmd_init(opts2).expect("remove hook");

    let after = read_json(&root.join(".claude/settings.json"));
    let arr = after["hooks"][SESSION_START_KEY].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["hooks"][0]["command"].as_str().unwrap(), "echo hi");
}

#[test]
fn with_hook_replaces_stale_managed_entry() {
    let scratch = scratch();
    let root = scratch.path();
    let mut opts = claude_opts(root);
    opts.with_hook = true;
    opts.binary_path = Some(PathBuf::from("/old/path/graphatlas"));
    cmd_init(opts).expect("first install");

    let mut opts2 = claude_opts(root);
    opts2.with_hook = true;
    opts2.binary_path = Some(PathBuf::from("/new/path/graphatlas"));
    cmd_init(opts2).expect("second install");

    let settings = read_json(&root.join(".claude/settings.json"));
    let arr = settings["hooks"][SESSION_START_KEY].as_array().unwrap();
    let managed: Vec<&Value> = arr
        .iter()
        .filter(|e| e[MANAGED_TAG_KEY].as_str() == Some(MANAGED_TAG_VALUE))
        .collect();
    assert_eq!(managed.len(), 1);
    let cmd = managed[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.starts_with("/new/path/graphatlas"));
}

// ===================================================================
// Per-platform smoke (Cursor / Cline / Codex / Gemini)
// ===================================================================

fn platform_opts(root: &Path, platform: Platform) -> InitOptions {
    InitOptions {
        project_root: Some(root.to_path_buf()),
        platforms: vec![platform],
        yes: true,
        binary_path: Some(PathBuf::from("/usr/local/bin/graphatlas")),
        ..InitOptions::default()
    }
}

/// Returns the set of platforms that need a HomeGuard wrap because
/// the install flow writes to HOME-global config paths (real user
/// filesystem leak otherwise). Audit:
/// - Cline: MCP register goes to per-OS Code/User/globalStorage
/// - Codex: reindex hook config goes to $HOME/.codex/config.toml
/// - Windsurf: MCP register goes to $HOME/.codeium/windsurf/mcp_config.json
#[allow(dead_code)]
fn _platforms_with_home_writes() -> &'static [Platform] {
    &[Platform::Cline, Platform::CodexCli, Platform::Windsurf]
}

#[test]
fn cursor_writes_mdc_rule_and_project_mcp() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::Cursor)).expect("init cursor");

    let mdc = root.join(".cursor/rules/graphatlas.mdc");
    let body = fs::read_to_string(&mdc).expect("mdc exists");
    assert!(body.starts_with("---"), "frontmatter missing: {body:?}");
    assert!(body.contains("alwaysApply: true"));
    assert!(body.contains("ga_callers"));

    let mcp = read_json(&root.join(".cursor/mcp.json"));
    assert_eq!(
        mcp["mcpServers"]["graphatlas"]["command"],
        json!("/usr/local/bin/graphatlas")
    );
}

#[test]
fn cline_writes_rules_file() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::Cline)).expect("init cline");

    // Directory form: .clinerules/graphatlas.md (sibling to .clinerules/hooks/).
    let rules = root.join(".clinerules/graphatlas.md");
    let body = fs::read_to_string(&rules).expect(".clinerules/graphatlas.md");
    assert!(body.contains("<!-- graphatlas:begin -->"));
    assert!(body.contains("<!-- graphatlas:end -->"));
    assert!(body.contains("ga_callers"));
}

#[test]
fn codex_writes_toml_mcp_and_agents_md() {
    let scratch = scratch();
    let root = scratch.path();
    // Codex reindex hook writes to $HOME/.codex/config.toml — guard
    // HOME so the test doesn't pollute the real user's Codex config.
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::CodexCli)).expect("init codex");

    // Project-local .codex/config.toml — verify TOML shape.
    let toml = read_toml(&root.join(".codex/config.toml"));
    let server = toml.get("mcp_servers").and_then(|t| t.get("graphatlas"));
    assert!(server.is_some(), "expected [mcp_servers.graphatlas]: {toml:?}");
    let cmd = server
        .and_then(|t| t.get("command"))
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(cmd, "/usr/local/bin/graphatlas");

    let agents = fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md");
    assert!(agents.contains("<!-- graphatlas:begin -->"));
    assert!(agents.contains("ga_callers"));
}

#[test]
fn gemini_writes_settings_and_gemini_md() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::GeminiCli)).expect("init gemini");

    let mcp = read_json(&root.join(".gemini/settings.json"));
    assert_eq!(
        mcp["mcpServers"]["graphatlas"]["command"],
        json!("/usr/local/bin/graphatlas")
    );

    let body = fs::read_to_string(root.join("GEMINI.md")).expect("GEMINI.md");
    assert!(body.contains("<!-- graphatlas:begin -->"));
    assert!(body.contains("ga_callers"));
}

#[test]
fn windsurf_writes_mcp_and_rules_file() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::Windsurf)).expect("init windsurf");

    let mcp = read_json(&root.join(".codeium/windsurf/mcp_config.json"));
    assert_eq!(
        mcp["mcpServers"]["graphatlas"]["command"],
        json!("/usr/local/bin/graphatlas")
    );

    let rules = fs::read_to_string(root.join(".windsurfrules")).expect(".windsurfrules");
    assert!(rules.contains("<!-- graphatlas:begin -->"));
    assert!(rules.contains("ga_callers"));
}

#[test]
fn continue_writes_per_server_mcp_file() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::Continue)).expect("init continue");

    let mcp = read_json(&root.join(".continue/mcpServers/graphatlas.json"));
    assert_eq!(
        mcp["mcpServers"]["graphatlas"]["command"],
        json!("/usr/local/bin/graphatlas")
    );
    // No instruction file expected for Continue (MCP-only).
    assert!(!root.join("GEMINI.md").exists());
}

#[test]
fn zed_writes_context_servers_with_source_custom() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::Zed)).expect("init zed");

    let settings = read_json(&root.join(".zed/settings.json"));
    // Zed uses `context_servers`, NOT `mcpServers`.
    let entry = &settings["context_servers"]["graphatlas"];
    assert_eq!(entry["command"], json!("/usr/local/bin/graphatlas"));
    assert_eq!(entry["source"], json!("custom"));
    // No mcpServers root expected.
    assert!(settings.get("mcpServers").is_none());
}

#[test]
fn multi_platform_install_writes_all_eight() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    let mut opts = claude_opts(root);
    opts.platforms = Platform::ALL.to_vec();
    cmd_init(opts).expect("init multi");

    // Claude Code
    assert!(root.join(".claude/skills/graphatlas.md").exists());
    assert!(root.join("CLAUDE.md").exists());
    // Cursor
    assert!(root.join(".cursor/rules/graphatlas.mdc").exists());
    assert!(root.join(".cursor/mcp.json").exists());
    // Cline (project-local rules + hook script under .clinerules/)
    assert!(root.join(".clinerules/graphatlas.md").exists());
    assert!(root.join(".clinerules/hooks/PostToolUse").exists());
    // Codex
    assert!(root.join(".codex/config.toml").exists());
    assert!(root.join("AGENTS.md").exists());
    // Gemini
    assert!(root.join(".gemini/settings.json").exists());
    assert!(root.join("GEMINI.md").exists());
    // Windsurf — user-global MCP under fake $HOME + project rules.
    assert!(root.join(".codeium/windsurf/mcp_config.json").exists());
    assert!(root.join(".windsurfrules").exists());
    // Continue
    assert!(root.join(".continue/mcpServers/graphatlas.json").exists());
    // Zed
    assert!(root.join(".zed/settings.json").exists());
}

#[test]
fn cursor_idempotent_rerun() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::Cursor)).unwrap();
    cmd_init(platform_opts(root, Platform::Cursor)).unwrap();
    let body = fs::read_to_string(root.join(".cursor/rules/graphatlas.mdc")).unwrap();
    assert_eq!(body.matches("alwaysApply: true").count(), 1);
}

#[test]
fn cline_idempotent_rerun() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::Cline)).unwrap();
    cmd_init(platform_opts(root, Platform::Cline)).unwrap();
    let body = fs::read_to_string(root.join(".clinerules/graphatlas.md")).unwrap();
    assert_eq!(body.matches("<!-- graphatlas:begin -->").count(), 1);
}

#[test]
fn codex_preserves_user_agents_md_content() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    fs::write(root.join("AGENTS.md"), "# Project rules\n\nDon't touch this.\n").unwrap();
    cmd_init(platform_opts(root, Platform::CodexCli)).unwrap();
    let after = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(after.contains("Don't touch this."));
    assert!(after.contains("<!-- graphatlas:begin -->"));
}

// ===================================================================
// Interactive numbered-toggle picker
// ===================================================================

fn detect_empty() -> HashSet<Platform> {
    HashSet::new()
}

fn detect_only(p: Platform) -> HashSet<Platform> {
    let mut s = HashSet::new();
    s.insert(p);
    s
}

#[test]
fn interactive_done_with_default_detected_selection() {
    // Detected: claude-code only. Pressing "done" right away → picks
    // just claude-code (the detected default).
    let detected = detect_only(Platform::ClaudeCode);
    let answers = b"done\nn\ny\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let (plats, hook) =
        graphatlas::cmd_init::interactive_pick_platforms_io(&detected, &mut reader, &mut writer)
            .unwrap()
            .expect("user confirmed");
    assert_eq!(plats, vec![Platform::ClaudeCode]);
    assert!(!hook);
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("Selected: claude-code"));
}

#[test]
fn interactive_toggle_numbers_adds_platforms() {
    // Start: no detection. Toggle 2 (cursor) and 4 (codex). Confirm.
    let detected = detect_empty();
    let answers = b"2 4\ndone\ny\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let (plats, _) =
        graphatlas::cmd_init::interactive_pick_platforms_io(&detected, &mut reader, &mut writer)
            .unwrap()
            .expect("confirmed");
    assert_eq!(plats, vec![Platform::Cursor, Platform::CodexCli]);
}

#[test]
fn interactive_all_shortcut_selects_everything() {
    let answers = b"all\ndone\nn\ny\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let (plats, _) = graphatlas::cmd_init::interactive_pick_platforms_io(
        &detect_empty(),
        &mut reader,
        &mut writer,
    )
    .unwrap()
    .expect("confirmed");
    assert_eq!(plats.len(), Platform::ALL.len());
}

#[test]
fn interactive_none_then_done_aborts() {
    let detected = detect_only(Platform::ClaudeCode);
    let answers = b"none\ndone\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let outcome =
        graphatlas::cmd_init::interactive_pick_platforms_io(&detected, &mut reader, &mut writer)
            .unwrap();
    assert!(outcome.is_none());
    let rendered = String::from_utf8_lossy(&writer);
    assert!(rendered.contains("Nothing selected"));
}

#[test]
fn interactive_decline_at_proceed_aborts() {
    let detected = detect_only(Platform::ClaudeCode);
    // Press done, decline hook, decline proceed.
    let answers = b"done\nn\nn\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let outcome =
        graphatlas::cmd_init::interactive_pick_platforms_io(&detected, &mut reader, &mut writer)
            .unwrap();
    assert!(outcome.is_none(), "user declined proceed");
}

#[test]
fn interactive_invalid_toggle_input_reprompts() {
    let detected = detect_only(Platform::ClaudeCode);
    // "xyz" rejected → re-prompt → "done" accepts.
    let answers = b"xyz\ndone\nn\ny\n";
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let _ = graphatlas::cmd_init::interactive_pick_platforms_io(
        &detected,
        &mut reader,
        &mut writer,
    )
    .unwrap()
    .expect("confirmed after retry");
    let rendered = String::from_utf8_lossy(&writer);
    assert!(
        rendered.contains("expected number"),
        "missing reprompt hint: {rendered}"
    );
}

#[test]
fn interactive_hook_prompt_only_when_claude_selected() {
    // Pick only cursor → hook prompt should be skipped (defaults false).
    let answers = b"2\ndone\ny\n"; // toggle 2 (cursor), done, proceed
    let mut reader = std::io::Cursor::new(answers);
    let mut writer: Vec<u8> = Vec::new();
    let (plats, hook) = graphatlas::cmd_init::interactive_pick_platforms_io(
        &detect_empty(),
        &mut reader,
        &mut writer,
    )
    .unwrap()
    .expect("confirmed");
    assert_eq!(plats, vec![Platform::Cursor]);
    assert!(!hook, "hook should default false when claude-code not picked");
    let rendered = String::from_utf8_lossy(&writer);
    assert!(
        !rendered.contains("Install Claude Code SessionStart hook"),
        "hook prompt should not appear: {rendered}"
    );
}

#[test]
fn interactive_eof_confirms_current_selection() {
    // EOF immediately → confirm with detected default (claude-code).
    let detected = detect_only(Platform::ClaudeCode);
    let mut reader = std::io::Cursor::new(&b""[..]);
    let mut writer: Vec<u8> = Vec::new();
    let (plats, hook) =
        graphatlas::cmd_init::interactive_pick_platforms_io(&detected, &mut reader, &mut writer)
            .unwrap()
            .expect("EOF confirms with current selection");
    assert_eq!(plats, vec![Platform::ClaudeCode]);
    assert!(!hook);
}

// ===================================================================
// Hook session-start reminder content
// ===================================================================

// ===================================================================
// Reindex hook auto-install (Claude Code / Cursor / Codex only)
// ===================================================================

#[test]
fn claude_init_installs_reindex_hook_by_default() {
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(claude_opts(root)).expect("init");
    let settings = read_json(&root.join(".claude/settings.json"));
    let post = settings["hooks"]["PostToolUse"]
        .as_array()
        .expect("PostToolUse array");
    let ga = post.iter().find(|e| {
        e["hooks"][0]["server"].as_str() == Some("graphatlas")
            && e["hooks"][0]["tool"].as_str() == Some("ga_reindex")
    });
    assert!(ga.is_some(), "expected GA PostToolUse entry: {settings}");
}

#[test]
fn no_reindex_hook_flag_skips_install() {
    let scratch = scratch();
    let root = scratch.path();
    let mut opts = claude_opts(root);
    opts.no_reindex_hook = true;
    cmd_init(opts).expect("init --no-reindex-hook");
    let settings = read_json(&root.join(".claude/settings.json"));
    assert!(
        settings["hooks"].get("PostToolUse").is_none(),
        "PostToolUse should be absent: {settings}"
    );
}

#[test]
fn cursor_init_installs_reindex_hook() {
    // Spec verified 2026-05: Cursor hooks live in .cursor/hooks.json
    // (NOT .cursor/mcp.json) and use camelCase `postToolUse` with a
    // shell `command` (NOT type:mcp_tool). Earlier code wrote the
    // wrong file + shape — Cursor silently ignored it.
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::Cursor)).expect("init cursor");
    let hooks = read_json(&root.join(".cursor/hooks.json"));
    assert_eq!(hooks["version"], json!(1));
    let post = hooks["hooks"]["postToolUse"]
        .as_array()
        .expect("postToolUse on cursor");
    let ga = post
        .iter()
        .find(|e| e["_managed_by"].as_str() == Some("graphatlas"))
        .expect("GA-managed Cursor hook");
    let cmd = ga["command"].as_str().unwrap();
    assert!(cmd.ends_with(" reindex"), "command was {cmd:?}");
}

#[test]
fn codex_init_installs_reindex_hook_toml() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::CodexCli)).expect("init codex");

    // Spec verified 2026-05: Codex hooks use `type = "command"` shell
    // (not `mcp_tool`). Entry has matcher + hooks[].command pointing
    // at `<binary> reindex`.
    let toml_doc = read_toml(&root.join(".codex/config.toml"));
    let post = toml_doc
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array())
        .expect("hooks.PostToolUse array in TOML");
    let ga = post
        .iter()
        .find(|e| e.get("_managed_by").and_then(|v| v.as_str()) == Some("graphatlas"))
        .expect("GA-managed Codex hook");
    let inner = ga.get("hooks").and_then(|h| h.as_array()).unwrap();
    assert_eq!(inner[0].get("type").and_then(|v| v.as_str()), Some("command"));
    let cmd = inner[0].get("command").and_then(|v| v.as_str()).unwrap();
    assert!(cmd.ends_with(" reindex"), "command was {cmd:?}");
}

#[test]
fn gemini_init_installs_aftertool_hook() {
    // Gemini CLI hook name is `AfterTool` (not PostToolUse — that's
    // Claude Code). Entry type is `command` (shell), not `mcp_tool`.
    let scratch = scratch();
    let root = scratch.path();
    cmd_init(platform_opts(root, Platform::GeminiCli)).expect("init gemini");
    let mcp = read_json(&root.join(".gemini/settings.json"));
    let after = mcp["hooks"]["AfterTool"]
        .as_array()
        .expect("AfterTool array");
    let ga = after
        .iter()
        .find(|e| e["_managed_by"].as_str() == Some("graphatlas"))
        .expect("GA-managed entry");
    assert_eq!(ga["hooks"][0]["type"], serde_json::json!("command"));
    let cmd = ga["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.ends_with(" reindex"), "command was {cmd:?}");
}

#[test]
fn cline_init_writes_executable_hook_script() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::Cline)).expect("init cline");
    let script = root.join(".clinerules/hooks/PostToolUse");
    let body = fs::read_to_string(&script).expect("hook script");
    assert!(body.starts_with("#!/usr/bin/env bash"));
    assert!(body.contains("graphatlas-managed"));
    assert!(body.contains(" reindex"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "script should be executable");
    }
}

#[test]
fn windsurf_init_installs_post_write_code_hook() {
    let scratch = scratch();
    let root = scratch.path();
    let _home = HomeGuard::set(root);
    cmd_init(platform_opts(root, Platform::Windsurf)).expect("init windsurf");

    let hooks = read_json(&root.join(".windsurf/hooks.json"));
    for key in ["post_write_code", "post_run_command"] {
        let arr = hooks["hooks"][key]
            .as_array()
            .unwrap_or_else(|| panic!("missing {key}: {hooks}"));
        let ga = arr
            .iter()
            .find(|e| e["_managed_by"].as_str() == Some("graphatlas"))
            .unwrap_or_else(|| panic!("no GA entry in {key}: {hooks}"));
        let cmd = ga["command"].as_str().unwrap();
        assert!(cmd.ends_with(" reindex"));
    }
}

#[test]
fn continue_and_zed_get_no_reindex_hook() {
    // Continue + Zed have no stable post-tool hook story → no hook is
    // written.
    for platform in [Platform::Continue, Platform::Zed] {
        let scratch = scratch();
        let root = scratch.path();
        cmd_init(platform_opts(root, platform)).expect("init");
        let any_hook = root.join(".windsurf/hooks.json").exists()
            || root.join(".clinerules/hooks").exists();
        assert!(!any_hook, "{platform:?} should not write hooks");
    }
}

#[test]
fn hook_session_start_subcommand_prints_reminder() {
    let body = graphatlas::cmd_hook::SESSION_START_REMINDER;
    assert!(body.contains("GraphAtlas code-graph available"));
    assert!(body.contains("ga_callers"));
    assert!(body.contains("ga_impact"));
}
