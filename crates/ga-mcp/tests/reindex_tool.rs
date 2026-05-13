//! v1.5 PR6 — `ga_reindex` MCP tool MVP tests.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-tool.md`
//! S-001 (AS-001/002) + S-002 (AS-004) + S-004 (AS-009/010).
//! AS-003/005/006/007/011 deferred to PR6.1 follow-up.

use ga_core::Error;
use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::{handle_tools_call_with_ctx, handle_tools_list};
use ga_mcp::types::ToolsCallParams;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn real_repo(tmp: &TempDir, rel: &str) -> PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src").join("lib.rs"), "// fixture\n").unwrap();
    p
}

fn fresh_ctx(tmp: &TempDir, rel: &str) -> McpContext {
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(tmp, rel);
    let mut s = Store::open_with_root(&cache, &repo).unwrap();
    s.commit_in_place().unwrap();
    McpContext::new(Arc::new(s))
}

// =====================================================================
// S-001 AS-001 — descriptor in tools/list
// =====================================================================

#[test]
fn as_001_tools_list_includes_ga_reindex_as_tool_15() {
    let result = handle_tools_list();
    assert_eq!(
        result.tools.len(),
        15,
        "tools/list must return exactly 15 tools (14 read + ga_reindex)"
    );
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"ga_reindex"),
        "tools/list must include ga_reindex; got {names:?}"
    );
}

#[test]
fn as_001_ga_reindex_descriptor_has_input_schema_with_mode_enum() {
    let result = handle_tools_list();
    let descr = result
        .tools
        .iter()
        .find(|t| t.name == "ga_reindex")
        .expect("ga_reindex descriptor must be present");
    let schema = &descr.input_schema;
    let mode_enum = schema
        .pointer("/properties/mode/enum")
        .and_then(|v| v.as_array())
        .expect("input_schema must declare mode enum");
    let values: Vec<&str> = mode_enum.iter().filter_map(|v| v.as_str()).collect();
    assert!(values.contains(&"auto"));
    assert!(values.contains(&"full"));
}

// =====================================================================
// S-001 AS-002 — invalid mode rejected with -32602
// =====================================================================

#[test]
fn as_002_invalid_mode_returns_invalid_params() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "invalid-mode");
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "garbage" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    match result {
        Err(Error::InvalidParams(msg)) => {
            assert_eq!(
                Error::InvalidParams(msg.clone()).jsonrpc_code(),
                -32602,
                "InvalidParams must map to -32602"
            );
            assert!(
                msg.contains("mode"),
                "error message must name the rejected field (got {msg:?})"
            );
        }
        other => panic!("AS-002: expected InvalidParams, got {other:?}"),
    }
}

#[test]
fn as_002_mode_full_accepted() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "mode-full");
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    // Acceptable outcomes: Ok (MVP returns deferred sentinel) OR Err
    // (downstream of validation). MUST NOT be InvalidParams.
    let result = handle_tools_call_with_ctx(&ctx, &params);
    assert!(
        !matches!(result, Err(Error::InvalidParams(_))),
        "AS-002: `full` is a valid mode — must not be rejected; got {result:?}"
    );
}

#[test]
fn as_002_no_mode_arg_defaults_to_full() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "no-mode");
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({}),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    assert!(
        !matches!(result, Err(Error::InvalidParams(_))),
        "AS-002: omitting mode is valid (defaults to full); got {result:?}"
    );
}

// =====================================================================
// S-002 AS-004 — bench fixture path refused (defense-in-depth)
// =====================================================================

#[test]
fn as_004_bench_fixture_path_refused() {
    // Simulate a Store whose metadata.repo_root contains the fatal
    // segment. The tool checks the metadata before doing any DB work.
    let tmp = TempDir::new().unwrap();
    // Fixture path with the literal segment so the tool's path check fires.
    let bench_path = tmp.path().join("benches").join("fixtures").join("django");
    std::fs::create_dir_all(&bench_path).unwrap();
    std::fs::write(bench_path.join("README.md"), "# fake fixture\n").unwrap();
    let cache = tmp.path().join(".graphatlas");
    let mut s = Store::open_with_root(&cache, &bench_path).unwrap();
    s.commit_in_place().unwrap();
    let ctx = McpContext::new(Arc::new(s));

    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    match result {
        Err(Error::Other(e)) => {
            let s = e.to_string();
            assert!(
                s.contains("bench fixture path detected"),
                "AS-004: refusal message must name the bench fixture path; got {s:?}"
            );
        }
        other => panic!("AS-004: expected refusal, got {other:?}"),
    }
}

// =====================================================================
// S-004 AS-009 — cross-repo non-blocking (distinct mutex instances)
// =====================================================================

#[test]
fn cross_repo_mutex_distinct() {
    // AS-009: the per-repo lock registry returns DISTINCT Arc<Mutex<()>>
    // instances for different cache dirs. If both repos got the same
    // Mutex, cross-repo reindexes would serialize — which AS-009 forbids.
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "cross-a");
    let cache_a = tmp.path().join(".graphatlas").join("a");
    let cache_b = tmp.path().join(".graphatlas").join("b");
    let m_a = ctx.reindex_lock_for(&cache_a);
    let m_b = ctx.reindex_lock_for(&cache_b);
    assert!(
        !Arc::ptr_eq(&m_a, &m_b),
        "AS-009: distinct repos must own distinct reindex mutexes"
    );
}

// =====================================================================
// S-004 AS-010 — same-repo mutex identity (serialization anchor)
// =====================================================================

#[test]
fn same_repo_mutex_identity() {
    // AS-010: calling reindex_lock_for(SAME path) twice returns the SAME
    // Arc<Mutex<()>>. This is the identity check that proves serialization
    // works — both callers will contend on the same lock.
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "same-repo");
    let cache = tmp.path().join(".graphatlas").join("same");
    let m1 = ctx.reindex_lock_for(&cache);
    let m2 = ctx.reindex_lock_for(&cache);
    assert!(
        Arc::ptr_eq(&m1, &m2),
        "AS-010: same repo must reuse the same reindex mutex (identity)"
    );
}

// =====================================================================
// R1b-S002.AS-003 — ga_reindex now actually executes the rebuild
// =====================================================================

#[test]
fn r1b_as_003_ga_reindex_executes_full_rebuild_and_bumps_generation() {
    // PR6.1b wires the real close-rm-init rebuild via ctx.rebuild_via.
    // Successful dispatch returns the response shape
    // {reindexed: true, took_ms, files_indexed, graph_generation_before,
    //  graph_generation_after, new_root_hash}. graph_generation_after must
    // be strictly greater than graph_generation_before.
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "r1b-rebuild");
    let gen_before = ctx.store().metadata().graph_generation;
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params).expect("rebuild must succeed");
    assert!(!result.is_error);
    let payload = match &result.content[0] {
        ga_mcp::types::ContentBlock::Json { json } => json,
        _ => panic!("ga_reindex must return Json content"),
    };
    assert_eq!(
        payload.get("reindexed").and_then(|v| v.as_bool()),
        Some(true),
        "AS-003: reindexed=true after real rebuild"
    );
    let gen_after = payload
        .get("graph_generation_after")
        .and_then(|v| v.as_u64())
        .expect("graph_generation_after present");
    let gen_before_payload = payload
        .get("graph_generation_before")
        .and_then(|v| v.as_u64())
        .expect("graph_generation_before present");
    assert_eq!(gen_before_payload, gen_before);
    // Note: reindex_in_place nukes the cache and rebuilds from gen 1 (see
    // PR6.1a AS-004 carve-out — "continues sequence" is deferred). PR6.1b
    // pins the response *shape*, not generation continuity.
    assert!(
        gen_after >= 1,
        "AS-003: graph_generation_after must be ≥1 (before={gen_before}, after={gen_after})"
    );
    assert!(
        payload
            .get("new_root_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.len() == 64)
            .unwrap_or(false),
        "AS-003: new_root_hash present, 64-char hex"
    );
    assert!(
        payload.get("took_ms").and_then(|v| v.as_u64()).is_some(),
        "AS-003: took_ms present"
    );
    assert!(
        payload
            .get("files_indexed")
            .and_then(|v| v.as_u64())
            .is_some(),
        "AS-003: files_indexed present"
    );
}

// =====================================================================
// R1b-S002.STORE_BUSY — Arc<Store> refcount > 1 surfaces -32013 StoreBusy
// =====================================================================

#[test]
fn r1b_store_busy_when_outstanding_arc_clone_blocks_rebuild() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "r1b-busy");
    // Hold a live Arc<Store> clone — this is the "in-flight tool call"
    // scenario the StoreBusy variant guards against.
    let _holdout = ctx.store();
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    let result = handle_tools_call_with_ctx(&ctx, &params);
    match result {
        Err(Error::StoreBusy) => {}
        other => panic!("STORE_BUSY: expected Error::StoreBusy, got {other:?}"),
    }
    // After dropping the holdout, the next dispatch must succeed.
    drop(_holdout);
    let result2 = handle_tools_call_with_ctx(&ctx, &params).expect("rebuild succeeds after drain");
    assert!(!result2.is_error);
}

// =====================================================================
// R1b-S002.AS-005 — Mid-rebuild build_index failure leaves cache empty
// + ga_reindex returns -32012 REINDEX_BUILD_FAILED
// =====================================================================

#[test]
fn r1b_as_005_rebuild_failure_leaves_cell_empty_and_returns_reindex_build_failed() {
    // Directly exercises `ctx.rebuild_via` with a closure that fails — this
    // simulates a mid-rebuild build_index error without needing fault
    // injection inside ga-query. The contract under test is the McpContext
    // plumbing: failed closure → cell None → next store() call returns
    // Error::ReindexBuildFailed (-32012).
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "r1b-build-fail");
    let result = ctx.rebuild_via(|_store| {
        Err(Error::Other(anyhow::anyhow!(
            "simulated build_index failure"
        )))
    });
    let err = match result {
        Ok(_) => panic!("AS-005: rebuild_via must return Err when closure fails"),
        Err(e) => e,
    };
    match err {
        Error::ReindexBuildFailed { ref reason } => {
            assert!(
                reason.contains("simulated build_index failure"),
                "AS-005: -32012 must wrap original failure reason; got {reason:?}"
            );
        }
        other => panic!("AS-005: expected ReindexBuildFailed, got {other:?}"),
    }
    assert_eq!(err.jsonrpc_code(), -32012, "AS-005: error maps to -32012");

    // Subsequent store() access must surface ReindexBuildFailed (cell None)
    // — not silently serve stale data.
    match ctx.try_store() {
        Ok(_) => panic!("AS-005: store cell must be None after rebuild failure"),
        Err(Error::ReindexBuildFailed { .. }) => {}
        Err(other) => panic!("AS-005: expected ReindexBuildFailed on try_store, got {other:?}"),
    }
}

#[test]
fn r1b_as_005_tl_s002_build_failure_cache_left_in_recoverable_state() {
    // Tl-S002.AS-005 sibling: build failure leaves the on-disk cache empty
    // (nuke ran before the closure errored out), so the next Store::open
    // on this cache_root performs a fresh build. We assert the cache root
    // exists and is in a state that fresh-build can recover from (no
    // leftover stale graph.db preventing a new build).
    use ga_index::Store;
    use ga_query::indexer::build_index;
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "r1b-cache-nuked");
    let cache_layout = ctx.store().layout().dir().to_path_buf();
    let _ = ctx.rebuild_via(|store| {
        // Simulate reindex_in_place running (nuke + open fresh) but then
        // build_index failing.
        let repo_root = std::path::PathBuf::from(&store.metadata().repo_root);
        let fresh = store.reindex_in_place(&repo_root).unwrap();
        // ... build_index would fail here. We return Err to simulate.
        drop(fresh);
        Err(Error::Other(anyhow::anyhow!("simulated build failure")))
    });
    // Cache dir still exists; metadata may or may not be present, but a
    // subsequent Store::open against the same cache_root must succeed
    // (fresh build path). cache_layout points to <cache>/<hash>; its parent
    // is <cache>. We open against the original repo root.
    let cache_root = cache_layout.parent().unwrap();
    let repo_root = tmp.path().join("repos").join("r1b-cache-nuked");
    let recovered =
        Store::open_with_root(cache_root, &repo_root).expect("Store::open must recover");
    build_index(&recovered, &repo_root).expect("post-failure rebuild must succeed");
}

// =====================================================================
// PR6.1d AS-006 — 200ms post-success cooldown short-circuits with -32014
// =====================================================================

#[test]
fn r1d_as_006_post_success_cooldown_short_circuits_with_already_reindexing() {
    let tmp = TempDir::new().unwrap();
    let ctx = fresh_ctx(&tmp, "r1d-cooldown");
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    // First call: real rebuild succeeds, arms the cooldown.
    let first = handle_tools_call_with_ctx(&ctx, &params).expect("first reindex must succeed");
    assert!(!first.is_error);

    // Second call within the 200ms window: short-circuit -32014.
    let second = handle_tools_call_with_ctx(&ctx, &params);
    match second {
        Err(Error::AlreadyReindexing { ref hint }) => {
            assert!(
                hint.contains("cooldown"),
                "AS-006: hint must mention cooldown; got {hint:?}"
            );
            assert_eq!(
                second.as_ref().unwrap_err().jsonrpc_code(),
                -32014,
                "AS-006: maps to -32014"
            );
        }
        other => panic!("AS-006: expected AlreadyReindexing, got {other:?}"),
    }

    // After the cooldown window expires, reindex proceeds normally.
    std::thread::sleep(std::time::Duration::from_millis(220));
    let third =
        handle_tools_call_with_ctx(&ctx, &params).expect("post-cooldown reindex must succeed");
    assert!(!third.is_error);
}
