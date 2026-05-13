//! v1.5 PR5 Staleness Phase C — pre-tool dispatch gate tests.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-staleness.md`
//! S-003 AS-007..012.
//!
//! Coverage:
//! - AS-007 fresh state → dispatch normally
//! - AS-008 stale + no allow_stale → STALE_INDEX error
//! - AS-009 stale + allow_stale → dispatch
//! - AS-011 500ms TTL absorbs query bursts
//! - AS-012 ga_reindex bypasses gate unconditionally

use ga_core::Error;
use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::ToolsCallParams;
use ga_parser::staleness::StalenessChecker;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn real_repo(tmp: &TempDir, rel: &str) -> PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    // Subdirectory needed so the Merkle root hash has dir entries to
    // anchor against. compute_root_hash excludes the repo_root itself
    // (depth 0) — only depth>=1 dirs feed into the hash.
    let sub = p.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("lib.rs"), "// fixture\n").unwrap();
    p
}

/// Build a committed Store + McpContext on a real fixture repo.
///
/// Post-S-004 wiring (2026-05-13): also runs `build_index` so the
/// File table has rows + sha256 populated. Without this, the Tier 2
/// gate added in S-004 would treat every file on disk as "newly
/// appeared" relative to the empty graph snapshot, firing
/// STALE_INDEX from Tier 2 before any Tier 1-specific assertion can
/// run. Indexed-fresh is the correct "fresh state" semantics now.
fn fresh_ctx(tmp: &TempDir, rel: &str) -> McpContext {
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(tmp, rel);
    let mut store = Store::open_with_root(&cache_root, &repo).unwrap();
    ga_query::indexer::build_index(&store, &repo).expect("build_index");
    store.commit_in_place().unwrap();
    McpContext::new(Arc::new(store))
}

// =====================================================================
// AS-012: ga_reindex bypasses staleness gate unconditionally
// =====================================================================

#[test]
fn as_012_ga_reindex_bypasses_staleness_gate_even_when_stale() {
    // The carve-out is checked BEFORE the staleness gate, so even with a
    // mismatched indexed_root_hash the ga_reindex tool name routes
    // straight to dispatch. Dispatch will reject with InvalidParams or
    // SymbolNotFound or another error because ga_reindex tool isn't
    // registered until PR6 — but it MUST NOT return STALE_INDEX (-32010).
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "reindex-bypass");

    // Mutate the on-disk repo to force drift — add a new file so the
    // parent dir mtime bumps (Merkle hashes dir mtimes, not file content).
    let repo_root = PathBuf::from(&ctx.store().metadata().repo_root);
    // Adding a file under a tracked subdir bumps that subdir's mtime,
    // which Merkle hashes (root itself is excluded — see merkle.rs:99).
    // Sleep briefly because some FS have 1s mtime resolution and the
    // initial commit may have just stamped the dir.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(repo_root.join("src").join("drift.rs"), b"// drifted\n").unwrap();
    ctx.staleness.invalidate_cache();

    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({}),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    // Whatever error variant comes back, it MUST NOT be StaleIndex.
    // Any other outcome is fine for this test — ga_reindex doesn't
    // exist as a registered tool yet (PR6 work), so we expect some
    // form of "unknown tool" error. The contract this test pins is
    // the negative one: no STALE_INDEX gate.
    if let Err(Error::StaleIndex { .. }) = result {
        panic!("AS-012 violation: ga_reindex returned StaleIndex; gate must bypass it")
    }
}

// =====================================================================
// AS-007: Fresh state → tool dispatches normally
// =====================================================================

#[test]
fn as_007_fresh_state_dispatches_normally() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "fresh-dispatch");
    // Use a real registered tool. ga_symbols accepts no required args by
    // default and returns an empty list on an empty graph.
    let params = ToolsCallParams {
        name: "ga_symbols".to_string(),
        arguments: json!({ "name": "anything" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    // Fresh: must NOT return StaleIndex. Either Ok or another error
    // (validation, etc.) is acceptable — we only assert the negative.
    assert!(
        !matches!(result, Err(Error::StaleIndex { .. })),
        "AS-007: fresh state must not emit StaleIndex; got {result:?}"
    );
}

// =====================================================================
// AS-008: Stale + no allow_stale → STALE_INDEX error
// =====================================================================

#[test]
fn as_008_stale_without_allow_stale_returns_stale_index() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "stale-fail-closed");
    // Drift: bump a depth-1 directory by adding a nested subdir under it.
    // Merkle hash includes depth >=1 dirs only (root excluded); modifying
    // an existing subdir's contents/mtime is the reliable way to force a
    // hash change. Sleep first to escape coarse mtime resolution.
    let repo_root = PathBuf::from(&ctx.store().metadata().repo_root);
    let indexed = ctx.store().metadata().indexed_root_hash.clone();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::create_dir_all(repo_root.join("src").join("nested_drift")).unwrap();
    ctx.staleness.invalidate_cache();

    // Sanity: confirm Merkle root actually drifted (otherwise the test
    // would be testing fixture limitations, not the gate).
    let cfg = ga_parser::merkle::MerkleConfig::default();
    let cur_bytes = ga_parser::merkle::compute_root_hash(&repo_root, &cfg).unwrap();
    let cur: String = cur_bytes.iter().map(|b| format!("{b:02x}")).collect();
    assert_ne!(
        indexed, cur,
        "fixture must produce a drifted Merkle root before testing the gate; \
         indexed={indexed} current={cur}"
    );

    let params = ToolsCallParams {
        name: "ga_symbols".to_string(),
        arguments: json!({ "name": "anything" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    match result {
        Err(Error::StaleIndex {
            indexed_root,
            current_root,
            ..
        }) => {
            assert_eq!(indexed_root.len(), 64, "indexed_root hex shape");
            assert_eq!(current_root.len(), 64, "current_root hex shape");
            assert_ne!(indexed_root, current_root, "drift means hashes differ");
        }
        other => panic!("AS-008: expected StaleIndex, got {other:?}"),
    }
}

// =====================================================================
// AS-009: Stale + allow_stale: true → serve (dispatch through)
// =====================================================================

#[test]
fn as_009_stale_with_allow_stale_dispatches() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "stale-allow");
    // Drift via new file (definitive dir mtime change).
    let repo_root = PathBuf::from(&ctx.store().metadata().repo_root);
    // Adding a file under a tracked subdir bumps that subdir's mtime,
    // which Merkle hashes (root itself is excluded — see merkle.rs:99).
    // Sleep briefly because some FS have 1s mtime resolution and the
    // initial commit may have just stamped the dir.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(repo_root.join("src").join("drift.rs"), b"// drifted\n").unwrap();
    ctx.staleness.invalidate_cache();

    let params = ToolsCallParams {
        name: "ga_symbols".to_string(),
        arguments: json!({ "name": "anything", "allow_stale": true }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    // allow_stale: true must NOT emit StaleIndex — dispatch proceeds.
    assert!(
        !matches!(result, Err(Error::StaleIndex { .. })),
        "AS-009: allow_stale: true must bypass StaleIndex; got {result:?}"
    );
}

// =====================================================================
// AS-011: 500ms TTL cache absorbs query bursts
// =====================================================================

#[test]
fn as_011_500ms_ttl_absorbs_query_bursts() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "ttl-burst");

    // Force the first compute then verify subsequent calls hit the cache.
    let store_snap = ctx.store();
    let indexed_hex = &store_snap.metadata().indexed_root_hash;
    assert_eq!(
        indexed_hex.len(),
        64,
        "fresh build must have populated hash"
    );

    // Reset the staleness counter by constructing a fresh checker
    // dedicated to this test (avoids cross-test contamination).
    let dedicated = Arc::new(StalenessChecker::new(PathBuf::from(
        &ctx.store().metadata().repo_root,
    )));
    let test_ctx = McpContext::with_staleness(ctx.store().clone(), dedicated.clone());

    let before = dedicated.compute_count();
    for _ in 0..10 {
        let params = ToolsCallParams {
            name: "ga_symbols".to_string(),
            arguments: json!({ "name": "x" }),
        };
        let _ = handle_tools_call_with_ctx(&test_ctx, &params);
    }
    let after = dedicated.compute_count();
    let calls = after - before;
    assert!(
        calls <= 1,
        "AS-011: 10 burst calls within 500ms must compute root at most once, got {calls}"
    );
}
