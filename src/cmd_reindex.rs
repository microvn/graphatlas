//! `graphatlas reindex` — manual reindex CLI subcommand.
//!
//! Mirrors the MCP `ga_reindex` tool but without the MCP roundtrip,
//! so platform hooks that only support shell-command callbacks
//! (Cline, Gemini CLI, Windsurf) can trigger reindex via this binary.
//!
//! Behaviour: open cache for `repo` (defaults to cwd), reindex in-place,
//! build the graph, commit. Prints a one-line stats summary.

use anyhow::{Context, Result};
use ga_index::Store;
use std::path::{Path, PathBuf};

pub fn cmd_reindex(repo: Option<PathBuf>) -> Result<()> {
    let repo_root = match repo {
        Some(p) => p,
        None => std::env::current_dir().context("resolve cwd")?,
    };
    do_reindex(&repo_root)
}

pub fn do_reindex(repo_root: &Path) -> Result<()> {
    let started = std::time::Instant::now();
    // `Store::open` acquires the per-repo exclusive flock before any
    // mutation (see crates/ga-index/src/store.rs:91-101). If another
    // writer (typically `graphatlas mcp` server) holds the lock,
    // open() falls back to a read-only handle. We detect that here
    // and refuse rather than calling reindex_in_place on a read-only
    // store, which would corrupt-or-error halfway through.
    let store = Store::open(repo_root)
        .with_context(|| format!("open cache for {}", repo_root.display()))?;
    if store.is_read_only() {
        return Err(anyhow::anyhow!(
            "cache is locked by another writer (likely `graphatlas mcp`); \
             reindex skipped. Try again after the writer releases the lock."
        ));
    }
    let mut fresh = store
        .reindex_in_place(repo_root)
        .context("reindex_in_place")?;
    let stats = ga_query::indexer::build_index(&fresh, repo_root).context("build_index")?;
    fresh.commit_in_place().context("commit_in_place")?;
    let took_ms = started.elapsed().as_millis();
    println!(
        "Reindexed {}: {} files in {}ms",
        repo_root.display(),
        stats.files,
        took_ms
    );
    Ok(())
}
