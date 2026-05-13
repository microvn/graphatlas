//! `graphatlas mcp` subcommand implementation — extracted into lib so
//! integration tests can drive it without spawning subprocesses.
//!
//! infra:S-003 (v1.1-M0) wires rmcp 1.5 stdio transport. This module adds
//! the missing step from the first S-003 landing: indexing the repo BEFORE
//! handing stdout to the MCP loop (Phase A review finding C-1).
//!
//! Store::open_with_root applies schema DDL but does NOT populate the
//! graph. On `OpenOutcome::FreshBuild` / `Rebuild*`, the caller must run
//! `build_index` explicitly. Without this, tool calls against a
//! newly-cloned repo return empty results silently.
//!
//! Public entry `prepare_store_for_mcp` returns an `Arc<Store>` ready for
//! `ga_mcp::run_stdio` — ensuring the graph is non-empty when the first
//! `tools/call` arrives from the MCP client.

use anyhow::{Context, Result};
use ga_index::{OpenOutcome, Store};
use std::path::Path;
use std::sync::Arc;

/// Open the cache at `cache_root` for the repo at `repo_root`. If the
/// store opened in any state other than `Resumed` (i.e. fresh build or
/// rebuild-triggered), run `build_index` so downstream MCP tool calls
/// see a populated graph.
///
/// Returns the committed `Arc<Store>` ready for `ga_mcp::run_stdio`.
pub fn prepare_store_for_mcp(cache_root: &Path, repo_root: &Path) -> Result<Arc<Store>> {
    // v1.5 PR2 foundation S-003 (Phase A3) — refuse to serve MCP on bench
    // fixture paths. A reindex run against `benches/fixtures/<repo>` would
    // corrupt M1/M2/M3 gate baselines (submodule HEAD drift). The guard
    // catches the most common foot-gun: user `cd benches/fixtures/django`
    // and types `graphatlas mcp`. Real intentional override (no current
    // legitimate use case) is documented in the error message.
    if is_bench_fixture_path(repo_root) {
        anyhow::bail!(
            "MCP refuses to serve on bench fixture path. Reindex would corrupt \
             M1/M2/M3 gates.\nDetected segment 'benches/fixtures/' in canonical \
             path: {}\nRun on user project root instead.",
            repo_root.display()
        );
    }

    let mut store = Store::open_with_root(cache_root, repo_root)
        .with_context(|| format!("Store::open_with_root({cache_root:?}, {repo_root:?})"))?;

    match store.outcome() {
        OpenOutcome::Resumed => {
            // Cache already built — skip indexing (Foundation AS-007). Seal
            // the DB to READ_ONLY + downgrade flock so other Claude Code
            // terminals can attach concurrently against the same cache.
            store
                .seal_for_serving()
                .context("seal_for_serving on Resumed path")?;
        }
        OpenOutcome::AttachedReadOnly { writer_generation } => {
            // Another graphatlas instance owns the writer lock for this repo
            // (typical case: multiple Claude Code terminals in the same cwd).
            // We attached as a read-only reader against the committed cache —
            // serve query traffic, never index.
            eprintln!(
                "graphatlas mcp: attached read-only to writer (gen={writer_generation}); \
                 serving queries against committed cache"
            );
        }
        OpenOutcome::FreshBuild
        | OpenOutcome::RebuildSchemaMismatch { .. }
        | OpenOutcome::RebuildCrashRecovery { .. } => {
            eprintln!(
                "graphatlas mcp: indexing {} (cache not ready) …",
                repo_root.display()
            );
            ga_query::indexer::build_index(&store, repo_root)
                .context("initial build_index before MCP serve")?;
            store
                .commit_in_place()
                .context("commit metadata after build_index")?;
        }
    }

    Ok(Arc::new(store))
}

/// Entry point for the `graphatlas mcp` CLI subcommand. Walks cwd as repo
/// root, resolves the default cache dir, prepares the Store (indexing if
/// needed), then runs the rmcp stdio server until stdin closes.
pub fn cmd_mcp(cache_root: &Path) -> Result<()> {
    let repo_root = std::env::current_dir().context("std::env::current_dir")?;

    // Opt-in telemetry. Off unless GRAPHATLAS_TRACE=1; init failures
    // are stderr-warned and never block MCP serve. Boot is logged BEFORE
    // indexing so a slow-or-crashing initial build is still visible.
    ga_mcp::telemetry::Telemetry::install_global();
    if let Some(t) = ga_mcp::telemetry::Telemetry::global() {
        t.log_boot(
            &repo_root,
            serde_json::json!({
                "cache_root": cache_root.display().to_string(),
                "phase": "before_index",
            }),
        );
    }

    let store = prepare_store_for_mcp(cache_root, &repo_root)?;

    if let Some(t) = ga_mcp::telemetry::Telemetry::global() {
        let outcome = format!("{:?}", store.outcome());
        t.log_boot(
            &repo_root,
            serde_json::json!({
                "cache_root": cache_root.display().to_string(),
                "phase": "ready",
                "store_outcome": outcome,
            }),
        );
    }

    // v1.5 PR8 — Layer 1 `.git/`-scoped FS watcher. Shares the same
    // McpContext with the rmcp handler so per-repo mutex + 200ms
    // cooldown serialize watcher-triggered reindexes against
    // tool-triggered ones. Boot failure (NotAGitRepo, bench fixture,
    // inotify exhausted) is logged-not-fatal — Layer 3 staleness gate
    // (PR5) still covers correctness when the watcher is disabled.
    let ctx = ga_mcp::context::McpContext::new(store);
    let _l1_watcher = ga_mcp::watcher::spawn_l1_watcher(ctx.clone(), repo_root.clone());

    ga_mcp::run_stdio_with_ctx(ctx).context("ga_mcp::run_stdio_with_ctx")?;
    Ok(())
}

/// v1.5 PR2 foundation S-003 (Phase A3) — return true when `p` resolves
/// to a path under a `benches/fixtures/` segment. Used by
/// `prepare_store_for_mcp` to refuse MCP service on bench submodule
/// directories where reindex would corrupt M1/M2/M3 gates.
///
/// Algorithm (per challenge H-8 fix):
/// 1. Attempt `std::fs::canonicalize` — resolves `..`, `.`, symlinks.
/// 2. On canonicalize error (non-existent path / slow network mount),
///    fall back to lexical-only normalization via `Path::components`.
/// 3. Compare path string for the segment `benches/fixtures/` (POSIX
///    forward-slash form). Windows paths normalized by replacing `\` →
///    `/` before the comparison.
///
/// False positive risk: a user folder literally named like
/// `~/work/benches/fixtures/notes` outside any git repo will match. The
/// upstream multi-voice review accepted that risk as low; if you hit it,
/// rename the directory or run MCP from a parent.
pub fn is_bench_fixture_path(p: &Path) -> bool {
    // Try canonical first; fall back to lexical clean.
    let canonical = std::fs::canonicalize(p).ok();
    let probe = canonical.as_deref().unwrap_or(p);

    // Normalize to forward-slash string for cross-platform comparison.
    let s = probe.to_string_lossy().replace('\\', "/");
    s.contains("/benches/fixtures/")
}

#[cfg(test)]
mod bench_fixture_guard_tests {
    use super::is_bench_fixture_path;
    use std::path::Path;

    #[test]
    fn fixture_path_with_segment_matches() {
        // Pure lexical match (canonicalize fails on non-existent path,
        // falls through to lexical normalization).
        assert!(is_bench_fixture_path(Path::new(
            "/work/graphatlas/benches/fixtures/django"
        )));
    }

    #[test]
    fn regular_path_does_not_match() {
        assert!(!is_bench_fixture_path(Path::new("/work/myproject")));
        assert!(!is_bench_fixture_path(Path::new("/home/alice/code/myapp")));
    }

    #[test]
    fn path_with_only_benches_not_fixtures_does_not_match() {
        // `benches/` alone is fine — only `benches/fixtures/` is the
        // sentinel segment.
        assert!(!is_bench_fixture_path(Path::new(
            "/work/myproject/benches/my_bench.rs"
        )));
    }

    #[test]
    fn nested_fixture_path_matches() {
        assert!(is_bench_fixture_path(Path::new(
            "/work/graphatlas/benches/fixtures/typescript/express"
        )));
    }
}
