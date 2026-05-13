//! infra:S-003 Phase A review finding [C-1] — cmd_mcp must index repo
//! before serving. Store::open_with_root does NOT index; without
//! build_index, every tool call returns empty silently.
//!
//! Regression: Phase A shipped cmd_mcp without this step — src/main.rs
//! :205-217 called Store::open_with_root then straight into run_stdio.

use graphatlas::mcp_cmd::prepare_store_for_mcp;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// Fresh repo, fresh cache — `prepare_store_for_mcp` must populate the
/// graph before returning. Evidence: post-return symbol count > 0.
#[test]
fn prepare_store_indexes_fresh_repo() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(
        &repo.join("a.py"),
        "def foo():\n    pass\n\ndef caller():\n    foo()\n",
    );

    let store = prepare_store_for_mcp(&cache, &repo).expect("prepare must succeed on fresh repo");

    // Assert graph non-empty by counting Symbol nodes — mirrors what an
    // MCP tools/call against ga_callers would need.
    let conn = store.connection().expect("open connection");
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN count(s)")
        .expect("count query");
    let rows: Vec<_> = rs.into_iter().collect();
    let mut symbol_count: i64 = 0;
    for row in rows {
        for val in row.into_iter() {
            if let lbug::Value::Int64(n) = val {
                symbol_count = n;
            }
        }
    }
    assert!(
        symbol_count >= 2,
        "C-1 regression: prepare_store_for_mcp must index fresh repo \
         before returning (expected ≥2 symbols foo/caller, got {symbol_count})"
    );
}

/// Resumed-path: once cache is committed, `prepare_store_for_mcp` must
/// NOT re-index. Regression: prior `Store::commit(self)` consumed the
/// store and MCP needs it live → metadata stayed at `index_state=Building`
/// → next boot saw CrashedBuilding → nuke graph.db + rebuild. Fix added
/// `Store::commit_in_place(&mut self)` so MCP path commits without
/// surrendering the store.
#[test]
fn prepare_store_skips_index_on_resumed_cache() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def foo():\n    pass\n");

    // First call: populates cache.
    let _store1 = prepare_store_for_mcp(&cache, &repo).expect("first prepare");
    drop(_store1); // release lock

    // Mutate source AFTER first index. If second call re-indexes, `bar`
    // would appear in the graph. If it correctly takes the Resumed path,
    // only `foo` remains.
    write(
        &repo.join("a.py"),
        "def foo():\n    pass\n\ndef bar():\n    pass\n",
    );

    let store2 = prepare_store_for_mcp(&cache, &repo).expect("second prepare");
    let conn = store2.connection().expect("open connection");
    let rs = conn
        .query("MATCH (s:Symbol {name: 'bar'}) RETURN count(s)")
        .expect("count query");
    let mut bar_count: i64 = 0;
    for row in rs.into_iter() {
        for val in row.into_iter() {
            if let lbug::Value::Int64(n) = val {
                bar_count = n;
            }
        }
    }
    assert_eq!(
        bar_count, 0,
        "Resumed cache must NOT re-index — `bar` appeared in graph \
         after second prepare, meaning FreshBuild path ran twice"
    );
}
