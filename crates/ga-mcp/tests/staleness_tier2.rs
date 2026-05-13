//! v1.5 S-004 Tier 2 BLAKE3 dirty-paths staleness check tests.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-staleness.md`
//! S-004 AS-013..017 (added 2026-05-13 to close the content-only edit gap
//! surfaced in Tier C smoke on /Volumes/Data/projects/mobilefolk/standup-bot).
//!
//! Coverage:
//! - AS-013 Tier 1 fresh + Tier 2 fresh → dispatch normally
//! - AS-014 Content-only edit caught by Tier 2 → STALE_INDEX (-32010) with
//!         `data.dirty_paths` populated
//! - AS-015 1s TTL cache absorbs query burst — dirty_check_count ≤1 across 10
//! - AS-016 `.git/index` mtime change invalidates Tier 2 cache early
//! - AS-017 Large-repo opt-out via GRAPHATLAS_DISABLE_DIRTY_CHECK=1

use ga_core::Error;
use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::ToolsCallParams;
use ga_query::indexer::build_index;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Build a fully-indexed fixture: real Python source files + build_index
/// (so File.sha256 column populated) + commit_in_place (so root_hash
/// populated). Required because Tier 2 reads `File.sha256` snapshot from
/// the live graph.
fn indexed_ctx(tmp: &TempDir, rel: &str) -> (McpContext, PathBuf) {
    let cache_root = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src").join("util.py"),
        "def foo():\n    pass\n\ndef caller():\n    foo()\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("src").join("lib.py"),
        "from .util import foo\n\ndef runner():\n    foo()\n",
    )
    .unwrap();
    let mut store = Store::open_with_root(&cache_root, &repo).unwrap();
    build_index(&store, &repo).expect("build_index");
    store.commit_in_place().expect("commit");
    (McpContext::new(Arc::new(store)), repo)
}

fn call_params() -> ToolsCallParams {
    ToolsCallParams {
        name: "ga_callers".to_string(),
        arguments: json!({ "symbol": "foo" }),
    }
}

// =====================================================================
// AS-013 — Tier 1 fresh + Tier 2 fresh → dispatch normally
// =====================================================================

#[test]
fn as_013_tier1_fresh_plus_tier2_fresh_dispatches_normally() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _repo) = indexed_ctx(&tmp, "tier2-happy");
    let result = handle_tools_call_with_ctx(&ctx, &call_params());
    assert!(
        !matches!(result, Err(Error::StaleIndex { .. })),
        "AS-013: clean fixture must NOT return STALE_INDEX; got {result:?}"
    );
    assert!(
        result.is_ok(),
        "AS-013: dispatch must succeed on fresh+fresh state; got {result:?}"
    );
}

// =====================================================================
// AS-014 — Content-only edit caught by Tier 2 → STALE_INDEX
// =====================================================================

#[test]
fn as_014_content_only_edit_caught_by_tier2_returns_stale_with_dirty_paths() {
    let tmp = TempDir::new().unwrap();
    let (ctx, repo) = indexed_ctx(&tmp, "tier2-gap-close");

    // Sleep past the existing 500ms Tier 1 TTL + 1s Tier 2 TTL (AS-015)
    // so the next gate call recomputes both. Without this the Tier 2
    // cache from an implicit warmup could mask the drift.
    std::thread::sleep(std::time::Duration::from_millis(1_100));

    // Simulate Tier C smoke scenario: terminal `echo >> src/util.py` —
    // bumps file content but on APFS/HFS+ the parent `src/` dir mtime
    // may not advance past resolution. Tier 1 Merkle bounded sample
    // misses this; Tier 2 sha256-vs-disk catches it.
    std::fs::write(
        repo.join("src").join("util.py"),
        "def foo():\n    return 42  # edited\n\ndef caller():\n    foo()\n",
    )
    .unwrap();

    let result = handle_tools_call_with_ctx(&ctx, &call_params());
    match result {
        Err(Error::StaleIndex {
            ref current_root,
            ref dirty_paths,
            ..
        }) => {
            // Tier 2 must populate the dirty_paths list with the
            // modified file path.
            assert!(
                dirty_paths.iter().any(|p| p.ends_with("util.py")),
                "AS-014: dirty_paths must include the modified file; got {dirty_paths:?}"
            );
            // current_root may be "tier2" marker OR the recomputed Merkle
            // (Tier 2 is permitted to short-circuit before the second
            // Merkle compute). Either is acceptable — we just need the
            // error to fire.
            let _ = current_root;
        }
        other => panic!("AS-014: expected StaleIndex with dirty_paths populated, got {other:?}"),
    }
}

// =====================================================================
// AS-015 — Tier 2 1s TTL cache absorbs query burst
// =====================================================================

#[test]
fn as_015_tier2_1s_ttl_cache_absorbs_burst() {
    let tmp = TempDir::new().unwrap();
    let (ctx, _repo) = indexed_ctx(&tmp, "tier2-ttl");

    let before = ctx.staleness.dirty_check_count();
    // 10 sequential calls within ~50ms — well under both Tier 1 500ms TTL
    // and Tier 2 1s TTL.
    for _ in 0..10 {
        let _ = handle_tools_call_with_ctx(&ctx, &call_params());
    }
    let after = ctx.staleness.dirty_check_count();
    let calls = after - before;
    assert!(
        calls <= 1,
        "AS-015: 10 burst calls must hit cache after first compute; got {calls} dirty checks"
    );
}

// =====================================================================
// AS-016 — `.git/index` mtime change invalidates Tier 2 cache early
// =====================================================================

#[test]
fn as_016_git_index_mtime_change_invalidates_tier2_cache_early() {
    // Pre-create `.git/index` BEFORE indexed_ctx so the
    // `indexed_root_hash` snapshot includes its mtime. Without this,
    // Tier 1 would fire stale (Merkle drift from baseline) before
    // Tier 2's cache invalidation logic even runs.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repos").join("tier2-git-invalidate");
    let git_dir = repo.join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::write(git_dir.join("index"), b"DIRC\x00\x00\x00\x02").unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src").join("util.py"), "def foo():\n    pass\n").unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let mut store = Store::open_with_root(&cache_root, &repo).unwrap();
    build_index(&store, &repo).expect("build_index");
    store.commit_in_place().expect("commit");
    let ctx = McpContext::new(Arc::new(store));

    // Prime Tier 1 + Tier 2 caches.
    let _ = handle_tools_call_with_ctx(&ctx, &call_params());
    let after_prime = ctx.staleness.dirty_check_count();
    assert!(
        after_prime >= 1,
        "first call must populate Tier 2 cache; got {after_prime}"
    );

    // Within Tier 1's 500ms TTL, mutate `.git/index`. Tier 1 cache stays
    // hot (no recompute, returns the cached fresh hash) — Tier 2's
    // mtime-key short-circuit is the ONLY mechanism that catches the
    // drift. AS-016 asserts that catch fires (tier2_counter advances).
    std::thread::sleep(std::time::Duration::from_millis(50));
    let pre_meta = std::fs::metadata(git_dir.join("index")).unwrap();
    let pre_mtime = pre_meta.modified().unwrap();
    // Loop until OS-reported mtime advances (1ns on ext4, 1µs on APFS;
    // some FS may need a sleep+touch retry).
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2_000);
    while std::time::Instant::now() < deadline {
        std::fs::write(git_dir.join("index"), b"DIRC\x00\x00\x00\x02ADD").unwrap();
        if std::fs::metadata(git_dir.join("index"))
            .unwrap()
            .modified()
            .unwrap()
            > pre_mtime
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = handle_tools_call_with_ctx(&ctx, &call_params());
    let after_invalidate = ctx.staleness.dirty_check_count();
    assert!(
        after_invalidate > after_prime,
        "AS-016: .git/index mtime change must invalidate Tier 2 cache; \
         before={after_prime}, after={after_invalidate}"
    );
}

// =====================================================================
// AS-017 — Large-repo opt-out via env var
// =====================================================================

#[test]
fn as_017_opt_out_disables_tier2_check() {
    // Tier 2 disabled via the atomic flag (production sets this from
    // env `GRAPHATLAS_DISABLE_DIRTY_CHECK=1` at MCP boot OR from the
    // auto-detect on >10k files). Even after a content-only edit the
    // gate must NOT return STALE_INDEX from Tier 2 — fall through to
    // L1+L2 workflow per spec carve-out.
    let tmp = TempDir::new().unwrap();
    let (ctx, repo) = indexed_ctx(&tmp, "tier2-opt-out");
    ctx.staleness.set_tier2_disabled(true);

    std::thread::sleep(std::time::Duration::from_millis(1_100));
    std::fs::write(
        repo.join("src").join("util.py"),
        "def foo():\n    return 99\n\ndef caller():\n    foo()\n",
    )
    .unwrap();

    let result = handle_tools_call_with_ctx(&ctx, &call_params());
    let was_tier2_stale = matches!(
        result,
        Err(Error::StaleIndex { ref dirty_paths, .. }) if !dirty_paths.is_empty()
    );
    assert!(
        !was_tier2_stale,
        "AS-017: with opt-out flag set, Tier 2 must skip; got {result:?}"
    );
}
