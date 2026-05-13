//! `ga init` — merge an `mcp.allowedTools` entry into the project's
//! `.claude/settings.json` so the agent can invoke `mcp__graphatlas__*`
//! without permission prompts.
//!
//! Coexists with the v1.5 PR7 PostToolUse reindex hook in the same
//! file: deep-merges only the `mcp.allowedTools` key. All other keys
//! (`hooks`, `permissions`, custom entries) are preserved.

use super::json_io::{atomic_write_json, lock_file, read_json_or_empty};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const ALLOWED_PREFIX: &str = "mcp__graphatlas__";

#[derive(Debug, PartialEq, Eq)]
pub enum PermissionsOutcome {
    Created(PathBuf),
    Added(PathBuf),
    AlreadyPresent(PathBuf),
}

pub fn install_permissions(project_root: &Path) -> Result<PermissionsOutcome> {
    let target = project_root.join(".claude").join("settings.json");
    // TOCTOU defense: hold advisory flock for the read→modify→write
    // window. Without this, parallel ga init runs (rare but possible)
    // could lose one write — last-rename wins.
    let _lock = lock_file(&target)?;
    let existed = target.exists();
    let mut doc = read_json_or_empty(&target)?;

    let root = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} root must be a JSON object", target.display()))?;

    let permissions = root
        .entry("permissions".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`permissions` must be a JSON object"))?;

    let allow = permissions
        .entry("allow".to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("`permissions.allow` must be an array"))?;

    let wildcard = format!("{ALLOWED_PREFIX}*");
    let already = allow
        .iter()
        .any(|v| v.as_str().map(|s| s == wildcard).unwrap_or(false));
    if already {
        return Ok(PermissionsOutcome::AlreadyPresent(target));
    }
    allow.push(Value::String(wildcard));

    atomic_write_json(&target, &doc)?;
    Ok(if existed {
        PermissionsOutcome::Added(target)
    } else {
        PermissionsOutcome::Created(target)
    })
}

pub fn uninstall_permissions(project_root: &Path) -> Result<bool> {
    let target = project_root.join(".claude").join("settings.json");
    if !target.exists() {
        return Ok(false);
    }
    let mut doc = read_json_or_empty(&target)?;
    let Some(root) = doc.as_object_mut() else {
        return Ok(false);
    };
    let Some(perms) = root.get_mut("permissions").and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let Some(allow) = perms.get_mut("allow").and_then(|v| v.as_array_mut()) else {
        return Ok(false);
    };
    let wildcard = format!("{ALLOWED_PREFIX}*");
    let before = allow.len();
    allow.retain(|v| v.as_str() != Some(&wildcard));
    if allow.len() == before {
        return Ok(false);
    }
    if allow.is_empty() {
        perms.remove("allow");
    }
    if perms.is_empty() {
        root.remove("permissions");
    }
    atomic_write_json(&target, &doc)?;
    Ok(true)
}
