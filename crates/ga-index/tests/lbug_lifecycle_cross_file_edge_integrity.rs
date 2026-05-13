//! v1.5 PR1a Phase F empirical test #1 — cross-file edge integrity under
//! per-file DELETE+INSERT cycle.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-empirical.md`
//! S-001 (AS-001 + AS-002 + AS-003 partial).
//!
//! **Purpose**: PR9 (incremental pipeline) plans to re-parse only changed
//! files: per-file `MATCH (s:Symbol {file: 'X'}) DETACH DELETE s` followed by
//! INSERT of the new Symbols + edges. If lbug 0.16.1 silently drops or
//! dangles cross-file CALLS edges through that cycle, PR9 cannot ship and
//! the engine must fall back to full-rebuild (`reindex-tool.md` AS-016).
//!
//! These tests are the empirical PASS/FAIL gate. Marked `#[ignore]` so they
//! don't run on every `cargo test`; CI emits a JSON artifact (S-001 AS-003)
//! consumed by PR9's build gate.

use ga_index::Store;
use std::path::Path;
use tempfile::TempDir;

fn fresh_store(tmp: &TempDir, repo: &str) -> Store {
    let cache_root = tmp.path().join(".graphatlas");
    Store::open_with_root(&cache_root, Path::new(repo)).unwrap()
}

/// Helper: seed 3 files + 3 symbols + 2 cross-file CALLS edges.
/// Returns nothing — the store is left in a known state for the test
/// to mutate.
fn seed_three_file_graph(store: &Store) {
    let conn = store.connection().unwrap();

    // 3 File nodes (PK = path).
    for path in ["a.py", "b.py", "c.py"] {
        let q = format!("CREATE (:File {{path: '{path}', lang: 'python', size: 100}})");
        conn.query(&q)
            .unwrap_or_else(|e| panic!("seed File {path}: {e}"));
    }

    // 3 Symbols, one per file. Symbol PK = id, has `file` STRING.
    for (id, file) in [("sa", "a.py"), ("sb", "b.py"), ("sc", "c.py")] {
        let q = format!(
            "CREATE (:Symbol {{id: '{id}', name: 'fn', file: '{file}', \
             kind: 'function', line: 1, line_end: 5}})"
        );
        conn.query(&q)
            .unwrap_or_else(|e| panic!("seed Symbol {id}: {e}"));
    }

    // 2 cross-file CALLS edges: a→b and c→b. Both target Symbol sb (file=b.py).
    for (src, dst, line) in [("sa", "sb", 2), ("sc", "sb", 3)] {
        let q = format!(
            "MATCH (s:Symbol {{id: '{src}'}}), (t:Symbol {{id: '{dst}'}}) \
             CREATE (s)-[:CALLS {{call_site_line: {line}}}]->(t)"
        );
        conn.query(&q)
            .unwrap_or_else(|e| panic!("seed CALLS {src}->{dst}: {e}"));
    }
}

/// Helper: count CALLS edges matching src_file → dst_file path predicates.
fn count_calls(store: &Store, src_file: &str, dst_file: &str) -> i64 {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (s:Symbol {{file: '{src_file}'}})-[r:CALLS]->(t:Symbol {{file: '{dst_file}'}}) \
         RETURN count(r)"
    );
    let rs = conn
        .query(&q)
        .unwrap_or_else(|e| panic!("count_calls {src_file}->{dst_file}: {e}"));
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return n;
        }
    }
    panic!("count_calls: no row returned for {src_file}->{dst_file}");
}

#[test]
#[ignore = "Phase F gate — run via CI artifact pipeline"]
fn as_001_outgoing_cross_file_edge_survives_delete_insert_cycle() {
    // AS-001: graph has (:Symbol {file:'a.py'})-[:CALLS]->(:Symbol {file:'b.py'}).
    // Test deletes all Symbol rows where file='a.py' (which DETACH-cascades
    // the outgoing CALLS edge), then re-inserts the Symbol + edge.
    // Post-cycle: the cross-file edge MUST be queryable again.
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/cross-file-as-001");
    seed_three_file_graph(&store);

    // Sanity: edge exists pre-cycle.
    assert_eq!(
        count_calls(&store, "a.py", "b.py"),
        1,
        "pre-cycle: expected 1 CALLS a→b"
    );

    let conn = store.connection().unwrap();

    // Phase 1: DETACH DELETE all Symbol rows where file='a.py'. Cascades
    // CALLS rows pointing FROM these symbols.
    conn.query("MATCH (s:Symbol {file: 'a.py'}) DETACH DELETE s")
        .expect("DETACH DELETE a.py Symbols");

    // Sanity: edge gone post-delete (Symbol sa no longer exists).
    assert_eq!(
        count_calls(&store, "a.py", "b.py"),
        0,
        "mid-cycle: expected 0 CALLS a→b after DETACH DELETE"
    );

    // Phase 2: re-INSERT Symbol sa + CALLS sa→sb.
    conn.query(
        "CREATE (:Symbol {id: 'sa', name: 'fn', file: 'a.py', \
         kind: 'function', line: 1, line_end: 5})",
    )
    .expect("re-INSERT Symbol sa");
    conn.query(
        "MATCH (s:Symbol {id: 'sa'}), (t:Symbol {id: 'sb'}) \
         CREATE (s)-[:CALLS {call_site_line: 2}]->(t)",
    )
    .expect("re-INSERT CALLS sa→sb");

    // AS-001 verdict: cross-file edge restored cleanly.
    assert_eq!(
        count_calls(&store, "a.py", "b.py"),
        1,
        "post-cycle: expected 1 CALLS a→b after DELETE+INSERT (AS-001 PASS condition)"
    );
}

#[test]
#[ignore = "Phase F gate — run via CI artifact pipeline"]
fn as_002_incoming_cross_file_edges_to_changed_file_preserved() {
    // AS-002: graph has 2 incoming edges to file b.py — sa→sb and sc→sb.
    // Test deletes only b.py's symbols + the incoming CALLS edges, then
    // re-inserts. Post-cycle: BOTH incoming edges must be queryable.
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/cross-file-as-002");
    seed_three_file_graph(&store);

    // Sanity: both incoming edges exist.
    assert_eq!(count_calls(&store, "a.py", "b.py"), 1, "pre-cycle: a→b");
    assert_eq!(count_calls(&store, "c.py", "b.py"), 1, "pre-cycle: c→b");

    let conn = store.connection().unwrap();

    // Phase 1: DETACH DELETE b.py's symbols. This cascades the incoming
    // CALLS rows (cascade semantics — what we're really testing).
    conn.query("MATCH (s:Symbol {file: 'b.py'}) DETACH DELETE s")
        .expect("DETACH DELETE b.py Symbols");

    // Sanity: both incoming edges gone post-delete.
    assert_eq!(count_calls(&store, "a.py", "b.py"), 0, "mid-cycle: 0 a→b");
    assert_eq!(count_calls(&store, "c.py", "b.py"), 0, "mid-cycle: 0 c→b");

    // Phase 2: re-INSERT Symbol sb + both incoming CALLS edges.
    conn.query(
        "CREATE (:Symbol {id: 'sb', name: 'fn', file: 'b.py', \
         kind: 'function', line: 1, line_end: 5})",
    )
    .expect("re-INSERT Symbol sb");
    conn.query(
        "MATCH (s:Symbol {id: 'sa'}), (t:Symbol {id: 'sb'}) \
         CREATE (s)-[:CALLS {call_site_line: 2}]->(t)",
    )
    .expect("re-INSERT CALLS sa→sb");
    conn.query(
        "MATCH (s:Symbol {id: 'sc'}), (t:Symbol {id: 'sb'}) \
         CREATE (s)-[:CALLS {call_site_line: 3}]->(t)",
    )
    .expect("re-INSERT CALLS sc→sb");

    // AS-002 verdict: BOTH incoming edges restored. Cascade + re-insert
    // is the bread-and-butter operation PR9 incremental relies on.
    assert_eq!(
        count_calls(&store, "a.py", "b.py"),
        1,
        "post-cycle: expected 1 CALLS a→b (AS-002 PASS condition)"
    );
    assert_eq!(
        count_calls(&store, "c.py", "b.py"),
        1,
        "post-cycle: expected 1 CALLS c→b (AS-002 PASS condition)"
    );
}
