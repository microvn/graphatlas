//! `ga init` — append/update a managed block in `CLAUDE.md` pointing
//! agents at the GraphAtlas skill + MCP tools.
//!
//! Idempotency via marker comments. On re-run, the block between
//! `<!-- graphatlas:begin -->` and `<!-- graphatlas:end -->` is
//! replaced wholesale; anything outside the markers is preserved.

use super::json_io::atomic_write_bytes;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const BEGIN_MARKER: &str = "<!-- graphatlas:begin -->";
pub const END_MARKER: &str = "<!-- graphatlas:end -->";

const MANAGED_BODY: &str = r#"## Code navigation

This repo has a pre-built GraphAtlas index. Prefer `ga_*` MCP tools
over Grep/Glob/Bash for symbol-level queries — the graph has typed
CALL / IMPORT / CONTAINS edges that grep cannot see, distinguishes
call sites from value references, and resolves polymorphic dispatch.

See `.claude/skills/graphatlas.md` for the full routing table.

Quick routing:

- `who calls X` → `ga_callers`
- `what does X call` → `ga_callees`
- `impact of changing X` → `ga_impact`
- `is renaming X to Y safe` → `ga_rename_safety`
- `where is X` → `ga_symbols`
- `architecture / orient me` → `ga_architecture`
- `dead code` → `ga_dead_code`

Use Grep/Bash only for non-code content (logs, configs, prose).
"#;

#[derive(Debug, PartialEq, Eq)]
pub enum ClaudeMdOutcome {
    Created(PathBuf),
    BlockAdded(PathBuf),
    BlockReplaced(PathBuf),
    Unchanged(PathBuf),
}

pub fn install_claudemd(project_root: &Path) -> Result<ClaudeMdOutcome> {
    let target = project_root.join("CLAUDE.md");
    let managed_block = render_block();

    if !target.exists() {
        let body = format!("{managed_block}\n");
        atomic_write_bytes(&target, body.as_bytes())?;
        return Ok(ClaudeMdOutcome::Created(target));
    }

    let current = std::fs::read_to_string(&target)?;
    match find_block_bounds(&current) {
        Some((start, end)) => {
            let existing_block = &current[start..end];
            if existing_block == managed_block {
                return Ok(ClaudeMdOutcome::Unchanged(target));
            }
            let mut next = String::with_capacity(current.len());
            next.push_str(&current[..start]);
            next.push_str(&managed_block);
            next.push_str(&current[end..]);
            atomic_write_bytes(&target, next.as_bytes())?;
            Ok(ClaudeMdOutcome::BlockReplaced(target))
        }
        None => {
            let mut next = current;
            if !next.ends_with('\n') {
                next.push('\n');
            }
            if !next.is_empty() && !next.ends_with("\n\n") {
                next.push('\n');
            }
            next.push_str(&managed_block);
            next.push('\n');
            atomic_write_bytes(&target, next.as_bytes())?;
            Ok(ClaudeMdOutcome::BlockAdded(target))
        }
    }
}

pub fn uninstall_claudemd(project_root: &Path) -> Result<bool> {
    let target = project_root.join("CLAUDE.md");
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
    // Collapse the double-blank line we left behind if any.
    let trimmed = next.trim_end_matches('\n').to_string() + "\n";
    atomic_write_bytes(&target, trimmed.as_bytes())?;
    Ok(true)
}

fn render_block() -> String {
    format!("{BEGIN_MARKER}\n{MANAGED_BODY}{END_MARKER}")
}

fn find_block_bounds(haystack: &str) -> Option<(usize, usize)> {
    let start = haystack.find(BEGIN_MARKER)?;
    let end_marker_start = haystack[start..].find(END_MARKER)? + start;
    let end = end_marker_start + END_MARKER.len();
    Some((start, end))
}
