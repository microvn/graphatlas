//! `ga init --with-hook` — install a Claude Code SessionStart hook
//! that runs `<binary> hook session-start` once per session and injects
//! a discovery protocol reminder so the agent prefers `ga_*` over
//! Grep/Glob/Bash for code navigation.
//!
//! Coexists with the v1.5 PR7 PostToolUse reindex hook: both live in
//! `.claude/settings.json` under different top-level hook keys.
//!
//! Idempotency: hook entries are tagged with `_managed_by: graphatlas`.
//! Re-install replaces only entries with that tag; remove drops only
//! those entries.

use super::json_io::{atomic_write_json, lock_file, read_json_or_empty};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const MANAGED_TAG_KEY: &str = "_managed_by";
pub const MANAGED_TAG_VALUE: &str = "graphatlas";
pub const SESSION_START_KEY: &str = "SessionStart";

#[derive(Debug, PartialEq, Eq)]
pub enum SessionHookOutcome {
    Created(PathBuf),
    Added(PathBuf),
    AlreadyPresent(PathBuf),
    Replaced(PathBuf),
}

/// Install the SessionStart hook entry. `binary_path` is the command
/// Claude Code will exec; pass `std::env::current_exe()` in production.
pub fn install_session_hook(project_root: &Path, binary_path: &Path) -> Result<SessionHookOutcome> {
    let target = project_root.join(".claude").join("settings.json");
    let _lock = lock_file(&target)?;
    let existed = target.exists();
    let mut doc = read_json_or_empty(&target)?;
    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", target.display()))?;

    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`hooks` must be a JSON object"))?;

    let session_start = hooks
        .entry(SESSION_START_KEY.to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`hooks.SessionStart` must be an array"))?;

    let desired = ga_session_start_entry(binary_path);
    let mut replaced = false;
    let mut removed = 0usize;
    session_start.retain(|entry| {
        if is_ga_managed_entry(entry) {
            removed += 1;
            false
        } else {
            true
        }
    });
    if removed > 0 {
        replaced = true;
    }

    // Check if an *identical* entry already exists outside our tag —
    // if so, leave it alone (user hand-rolled it).
    let already_unmanaged = session_start
        .iter()
        .any(|e| matches_command(e, binary_path));

    if already_unmanaged && !replaced {
        return Ok(SessionHookOutcome::AlreadyPresent(target));
    }

    session_start.push(desired);
    atomic_write_json(&target, &doc)?;

    Ok(match (existed, replaced) {
        (false, _) => SessionHookOutcome::Created(target),
        (true, true) => SessionHookOutcome::Replaced(target),
        (true, false) => SessionHookOutcome::Added(target),
    })
}

/// Remove only the GA-managed SessionStart hook entry. Other entries
/// (user-authored, other-tool managed) are preserved.
pub fn uninstall_session_hook(project_root: &Path) -> Result<bool> {
    let target = project_root.join(".claude").join("settings.json");
    if !target.exists() {
        return Ok(false);
    }
    let mut doc = read_json_or_empty(&target)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(false);
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let Some(session_start) = hooks
        .get_mut(SESSION_START_KEY)
        .and_then(|v| v.as_array_mut())
    else {
        return Ok(false);
    };
    let before = session_start.len();
    session_start.retain(|entry| !is_ga_managed_entry(entry));
    if session_start.len() == before {
        return Ok(false);
    }
    if session_start.is_empty() {
        hooks.remove(SESSION_START_KEY);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    atomic_write_json(&target, &doc)?;
    Ok(true)
}

fn ga_session_start_entry(binary_path: &Path) -> Value {
    json!({
        MANAGED_TAG_KEY: MANAGED_TAG_VALUE,
        "matcher": "*",
        "hooks": [{
            "type": "command",
            "command": format!("{} hook session-start", binary_path.to_string_lossy()),
        }],
    })
}

fn is_ga_managed_entry(entry: &Value) -> bool {
    entry
        .get(MANAGED_TAG_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s == MANAGED_TAG_VALUE)
        .unwrap_or(false)
}

fn matches_command(entry: &Value, binary_path: &Path) -> bool {
    let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else {
        return false;
    };
    let needle = format!("{} hook session-start", binary_path.to_string_lossy());
    hooks.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(|c| c == needle)
            .unwrap_or(false)
    })
}
