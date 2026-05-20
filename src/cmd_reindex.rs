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

pub fn cmd_reindex(repo: Option<PathBuf>, json_progress: bool) -> Result<()> {
    let repo_root = match repo {
        Some(p) => p,
        None => std::env::current_dir().context("resolve cwd")?,
    };
    do_reindex(&repo_root, json_progress)
}

/// Emit one NDJSON phase event on stdout when --json-progress is on.
/// Format must match `ga_server::jobs::consume_progress` (phase + percent).
fn emit_phase(json_progress: bool, phase: &str, percent: f32) {
    if !json_progress {
        return;
    }
    // Hand-rolled to avoid pulling serde_json into the binary's reindex
    // hot path. Phase strings are static identifiers — no escape needed.
    println!("{{\"phase\":\"{phase}\",\"percent\":{percent}}}");
}

pub fn do_reindex(repo_root: &Path, json_progress: bool) -> Result<()> {
    let started = std::time::Instant::now();
    emit_phase(json_progress, "opening", 5.0);
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
    emit_phase(json_progress, "indexing", 20.0);
    let mut fresh = store
        .reindex_in_place(repo_root)
        .context("reindex_in_place")?;
    emit_phase(json_progress, "graph", 60.0);
    let stats = ga_query::indexer::build_index(&fresh, repo_root).context("build_index")?;
    emit_phase(json_progress, "committing", 90.0);
    fresh.commit_in_place().context("commit_in_place")?;
    let took_ms = started.elapsed().as_millis();

    // ga-ui Spec A S-003 — post-commit metadata sidecar update.
    // Best-effort: any tally failure logs but doesn't fail reindex
    // (the dashboard prefers "0" over a broken reindex).
    let counts = ga_query::counts::compute_index_counts(&fresh, took_ms as u64);
    let health = ga_query::counts::compute_health_summary(&fresh);
    let layout = fresh.layout().clone();
    let mut meta = fresh.metadata().clone();
    if let Err(e) = meta.set_index_counts(counts, &layout) {
        tracing::warn!(target: "graphatlas::reindex", "set_index_counts: {e}");
    }
    if let Err(e) = meta.set_health_summary(health, &layout) {
        tracing::warn!(target: "graphatlas::reindex", "set_health_summary: {e}");
    }

    emit_phase(json_progress, "done", 100.0);
    println!(
        "Reindexed {}: {} files in {}ms",
        repo_root.display(),
        stats.files,
        took_ms
    );
    Ok(())
}
