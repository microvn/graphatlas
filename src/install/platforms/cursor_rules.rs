//! Cursor instruction surface — `.cursor/rules/graphatlas.mdc`.
//!
//! Cursor rules use MDC format: YAML frontmatter + markdown body.
//! `alwaysApply: true` makes the rule load on every chat turn (vs
//! `globs:` which only loads when files matching the glob are
//! attached). `description` is used by Cursor's agent when deciding
//! whether to load a non-alwaysApply rule.
//!
//! Spec verified against cursor.com/docs/context/mcp + community forum
//! guides (2026-05).

use super::{PreflightSummary, SimpleOutcome};
use crate::install::json_io::atomic_write_bytes;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const REL_PATH: &str = ".cursor/rules/graphatlas.mdc";

const FRONTMATTER: &str = "\
---
description: GraphAtlas code-graph routing — prefer ga_* MCP tools over Grep/Glob/Bash for symbol-level queries
alwaysApply: true
---

";

const ROUTING_BODY: &str = include_str!("../../../assets/routing-block.md");

fn content() -> Vec<u8> {
    let mut out = Vec::with_capacity(FRONTMATTER.len() + ROUTING_BODY.len());
    out.extend_from_slice(FRONTMATTER.as_bytes());
    out.extend_from_slice(ROUTING_BODY.as_bytes());
    out
}

pub fn install(project_root: &Path) -> Result<SimpleOutcome> {
    let target = project_root.join(REL_PATH);
    let body = content();
    if target.exists() {
        let current = std::fs::read(&target).unwrap_or_default();
        if current == body {
            return Ok(SimpleOutcome::Unchanged(target));
        }
        atomic_write_bytes(&target, &body)?;
        return Ok(SimpleOutcome::Updated(target));
    }
    atomic_write_bytes(&target, &body)?;
    Ok(SimpleOutcome::Created(target))
}

pub fn uninstall(project_root: &Path) -> Result<bool> {
    let target = project_root.join(REL_PATH);
    if !target.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&target)?;
    Ok(true)
}

pub fn preflight(project_root: &Path) -> PreflightSummary {
    let target = project_root.join(REL_PATH);
    let state = if target.exists() {
        "exists — will refresh"
    } else {
        "missing — will create"
    };
    PreflightSummary { target, state }
}

pub fn target_path(project_root: &Path) -> PathBuf {
    project_root.join(REL_PATH)
}
