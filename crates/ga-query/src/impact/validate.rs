//! Cluster C1 — AS-015 input validation + path-stem helper used by
//! test-discovery.

use super::types::ImpactRequest;
use ga_core::{Error, Result};

/// Returns Ok when the request names at least one real seed. Empty strings,
/// whitespace-only strings, and empty arrays all count as "absent" — this
/// matches the spec's intent ("at least one of … required") and saves
/// downstream clusters from re-checking degenerate input.
pub(super) fn validate_seed_input(req: &ImpactRequest) -> Result<()> {
    let has_symbol = req
        .symbol
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_changed_files = req
        .changed_files
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let has_diff = req
        .diff
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if has_symbol || has_changed_files || has_diff {
        Ok(())
    } else {
        Err(Error::InvalidParams(
            "at least one of changed_files/symbol/diff required".to_string(),
        ))
    }
}

/// Stem of a repo-relative path — everything after the last `/` and before
/// the last `.`. Returns `None` for paths that have no file component.
pub(super) fn file_stem(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}
