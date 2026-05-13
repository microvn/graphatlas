//! S-005 AS-024 — parallel re-parse via rayon.
//!
//! Given a list of relative file paths and a per-file parse closure, fan out
//! across rayon's thread pool and gather per-file results in input order.
//! Progress callback fires once per completion so higher layers (MCP
//! `ga_reindex` response) can stream progress events when the batch exceeds
//! the 5s spec threshold.

use ga_core::{Lang, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone)]
pub struct ReparseResult {
    pub path: PathBuf,
    pub lang: Lang,
    pub symbols: Vec<crate::ParsedSymbol>,
    pub bytes: u64,
}

/// Progress tick. Emitted once per completed file.
#[derive(Debug, Clone, Copy)]
pub struct ReparseProgress {
    pub completed: usize,
    pub total: usize,
}

/// Re-parse `paths` in parallel using `parse_fn` per file. Results are
/// returned in the same order as the input slice. `progress` (if provided)
/// fires on every completion; callers should sample / throttle if they only
/// want periodic updates.
pub fn parallel_reparse<F, P>(
    paths: &[PathBuf],
    parse_fn: F,
    progress: Option<P>,
) -> Vec<Result<ReparseResult>>
where
    F: Fn(&Path) -> Result<ReparseResult> + Send + Sync,
    P: Fn(ReparseProgress) + Send + Sync,
{
    if paths.is_empty() {
        return Vec::new();
    }

    let total = paths.len();
    let completed = AtomicUsize::new(0);

    paths
        .par_iter()
        .map(|p| {
            let r = parse_fn(p);
            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(cb) = &progress {
                cb(ReparseProgress {
                    completed: done,
                    total,
                });
            }
            r
        })
        .collect()
}
