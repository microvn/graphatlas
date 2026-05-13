//! Gemini CLI instruction surface — `GEMINI.md` at project root.
//!
//! Gemini CLI reads `GEMINI.md` files hierarchically: home dir, project
//! root, then tool-accessed dirs. We write only the project-root
//! GEMINI.md with a managed-block marker for idempotency + user
//! content preservation.
//!
//! Spec verified against geminicli.com/docs/cli/gemini-md/ (2026-05).

use super::{PreflightSummary, SimpleOutcome};
use crate::install::json_io::atomic_write_bytes;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const REL_PATH: &str = "GEMINI.md";
pub const BEGIN_MARKER: &str = "<!-- graphatlas:begin -->";
pub const END_MARKER: &str = "<!-- graphatlas:end -->";

const ROUTING_BODY: &str = include_str!("../../../assets/routing-block.md");

fn managed_block() -> String {
    format!("{BEGIN_MARKER}\n{ROUTING_BODY}{END_MARKER}")
}

pub fn install(project_root: &Path) -> Result<SimpleOutcome> {
    let target = project_root.join(REL_PATH);
    let block = managed_block();

    if !target.exists() {
        let body = format!("{block}\n");
        atomic_write_bytes(&target, body.as_bytes())?;
        return Ok(SimpleOutcome::Created(target));
    }

    let current = std::fs::read_to_string(&target)?;
    if let Some((start, end)) = find_block_bounds(&current) {
        let existing = &current[start..end];
        if existing == block {
            return Ok(SimpleOutcome::Unchanged(target));
        }
        let mut next = String::with_capacity(current.len());
        next.push_str(&current[..start]);
        next.push_str(&block);
        next.push_str(&current[end..]);
        atomic_write_bytes(&target, next.as_bytes())?;
        Ok(SimpleOutcome::Updated(target))
    } else {
        let mut next = current;
        if !next.ends_with('\n') {
            next.push('\n');
        }
        if !next.is_empty() && !next.ends_with("\n\n") {
            next.push('\n');
        }
        next.push_str(&block);
        next.push('\n');
        atomic_write_bytes(&target, next.as_bytes())?;
        Ok(SimpleOutcome::Updated(target))
    }
}

pub fn uninstall(project_root: &Path) -> Result<bool> {
    let target = project_root.join(REL_PATH);
    if !target.exists() {
        return Ok(false);
    }
    let current = std::fs::read_to_string(&target)?;
    let Some((start, end)) = find_block_bounds(&current) else {
        return Ok(false);
    };
    let mut next = String::with_capacity(current.len());
    next.push_str(&current[..start]);
    next.push_str(&current[end..]);
    let trimmed = next.trim_end_matches('\n').to_string();
    if trimmed.is_empty() {
        std::fs::remove_file(&target)?;
    } else {
        let body = format!("{trimmed}\n");
        atomic_write_bytes(&target, body.as_bytes())?;
    }
    Ok(true)
}

pub fn preflight(project_root: &Path) -> PreflightSummary {
    let target = project_root.join(REL_PATH);
    let state = if !target.exists() {
        "missing — will create"
    } else if std::fs::read_to_string(&target)
        .map(|s| s.contains(BEGIN_MARKER))
        .unwrap_or(false)
    {
        "managed block present — will refresh"
    } else {
        "exists — will append managed block"
    };
    PreflightSummary { target, state }
}

pub fn target_path(project_root: &Path) -> PathBuf {
    project_root.join(REL_PATH)
}

fn find_block_bounds(haystack: &str) -> Option<(usize, usize)> {
    let start = haystack.find(BEGIN_MARKER)?;
    let end_marker_start = haystack[start..].find(END_MARKER)? + start;
    let end = end_marker_start + END_MARKER.len();
    Some((start, end))
}
