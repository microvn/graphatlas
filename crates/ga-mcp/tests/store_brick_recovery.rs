//! Regression: store cell bricks to None → read tools panic → MCP client hangs
//! forever. docs/investigate/mcp-store-brick-hang-2026-06-21.md
//!
//! Three invariants this pins (one per recommended action):
//!   1. A `rebuild_via` closure that fails with a BUSY/peer-lock signature must
//!      NOT brick the cell — the on-disk graph is still valid, so reopen it
//!      read-only and keep serving (action 1).
//!   2. When the cell IS legitimately None (genuine build failure), a read-tool
//!      dispatch must return `Err(ReindexBuildFailed)` gracefully, NOT panic —
//!      the panic is what hangs the rmcp client (action 2).
//!   3. `ga_reindex` must be able to rebuild a bricked (None) cell — it is the
//!      documented recovery path and must not itself panic (action 3).

use ga_core::Error;
use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::ToolsCallParams;
use serde_json::json;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn fixture(tmp: &TempDir) -> PathBuf {
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), b"pub fn hello() {}\n").unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    repo
}

/// Build + commit + seal a fixture, return a serving McpContext (cell = Some).
fn served_ctx(tmp: &TempDir) -> McpContext {
    let cache = tmp.path().join(".graphatlas");
    let repo = fixture(tmp);
    let mut store = Store::open_with_root(&cache, &repo).unwrap();
    ga_query::indexer::build_index(&store, &repo).unwrap();
    store.commit_in_place().unwrap();
    store.seal_for_serving().unwrap();
    McpContext::new(Arc::new(store))
}

#[test]
fn busy_rebuild_failure_self_heals_instead_of_bricking() {
    let tmp = TempDir::new().unwrap();
    let ctx = served_ctx(&tmp);

    // Closure mimics losing a multi-server reindex race: a peer holds the
    // writer lock. The on-disk graph is intact. Pre-fix, rebuild_via leaves
    // the cell None and the server bricks.
    let r = ctx.rebuild_via(|store| {
        drop(store); // release our handle/flock, as reindex_in_place would
        Err(Error::ReindexBuildFailed {
            reason: "peer process holds writer lock".to_string(),
        })
    });

    assert!(r.is_err(), "busy closure returns an error");
    assert!(
        ctx.try_store().is_ok(),
        "busy-branch rebuild must reopen read-only and self-heal, not brick the cell"
    );
}

#[test]
fn bricked_cell_dispatch_returns_error_not_panic() {
    let tmp = TempDir::new().unwrap();
    let ctx = served_ctx(&tmp);

    // Genuine (non-busy) build failure → cell is legitimately None.
    let r = ctx.rebuild_via(|store| {
        drop(store);
        Err(Error::ReindexBuildFailed {
            reason: "simulated genuine build failure".to_string(),
        })
    });
    assert!(matches!(r, Err(Error::ReindexBuildFailed { .. })));
    assert!(
        ctx.try_store().is_err(),
        "a genuine build failure leaves the cell None"
    );

    // The bug: handlers.rs:70 `ctx.store()` panics on a None cell, the panic
    // unwinds the rmcp call_tool task, no response is sent, client hangs.
    let params = ToolsCallParams {
        name: "ga_architecture".to_string(),
        arguments: json!({ "max_modules": 1 }),
    };
    let res = catch_unwind(AssertUnwindSafe(|| {
        handle_tools_call_with_ctx(&ctx, &params)
    }));

    assert!(
        res.is_ok(),
        "dispatch on a bricked cell must NOT panic (panic = client hang)"
    );
    assert!(
        matches!(res.unwrap(), Err(Error::ReindexBuildFailed { .. })),
        "must surface ReindexBuildFailed gracefully"
    );
}

#[test]
fn ga_reindex_recovers_a_bricked_cell() {
    let tmp = TempDir::new().unwrap();
    let ctx = served_ctx(&tmp);

    // Brick the cell (genuine build failure, on-disk left intact so recovery
    // is purely a cell-state problem).
    let _ = ctx.rebuild_via(|store| {
        drop(store);
        Err(Error::ReindexBuildFailed {
            reason: "simulated genuine build failure".to_string(),
        })
    });
    assert!(ctx.try_store().is_err(), "cell is bricked before recovery");

    // ga_reindex is the documented recovery escape-hatch. Pre-fix it reads
    // `ctx.store().metadata()` at reindex.rs:100 → panics on the None cell,
    // so recovery is impossible without restarting the server.
    let params = ToolsCallParams {
        name: "ga_reindex".to_string(),
        arguments: json!({ "mode": "full" }),
    };
    let res = catch_unwind(AssertUnwindSafe(|| {
        handle_tools_call_with_ctx(&ctx, &params)
    }));

    assert!(res.is_ok(), "ga_reindex on a bricked cell must NOT panic");
    let dispatch = res.unwrap();
    assert!(
        dispatch.is_ok(),
        "ga_reindex must rebuild the bricked cell, got {:?}",
        dispatch.err()
    );
    assert!(
        ctx.try_store().is_ok(),
        "store cell must be repopulated after ga_reindex recovery"
    );
}
