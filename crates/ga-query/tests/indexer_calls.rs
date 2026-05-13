//! Tools S-001 cluster B — indexer writes CALLS edges to the graph.
//!
//! Within-file resolution only: when `bar()` contains a call to `foo()` and
//! `foo` is defined in the same file, emit CALLS(bar_id → foo_id). Cross-file
//! resolution is a later cluster.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn bar_calling_foo_creates_calls_edge_within_same_file() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def foo():\n    pass\n\ndef bar():\n    foo()\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(stats.calls_edges >= 1, "expected CALLS edge, got {stats:?}");

    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (caller:Symbol)-[:CALLS]->(callee:Symbol {name: 'foo'}) \
             RETURN caller.name",
        )
        .unwrap();
    let caller_names: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(caller_names, vec!["bar".to_string()], "{caller_names:?}");
}

#[test]
fn unresolvable_callee_produces_external_edge() {
    // `bar()` calls `external_lib_func` which isn't defined in this file.
    // Tools S-002 cluster B evolution: instead of dropping, the indexer
    // synthesizes a single external Symbol node (kind='external') and writes
    // the CALLS edge — AS-004 needs `external: true` to reach the LLM.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def bar():\n    external_lib_func()\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.calls_edges >= 1,
        "unresolvable callee must now create an external edge: {stats:?}"
    );

    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (:Symbol)-[:CALLS]->(callee:Symbol) \
             WHERE callee.name = 'external_lib_func' \
             RETURN callee.kind",
        )
        .unwrap();
    let kinds: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec!["external".to_string()], "{kinds:?}");
}

#[test]
fn module_level_call_has_no_caller_edge() {
    // print() at top level → no enclosing symbol → no CALLS edge.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def foo(): pass\nfoo()\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(
        stats.calls_edges, 0,
        "module-level call has no caller to draw an edge from"
    );
}

#[test]
fn multiple_callers_all_recorded() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller_a(): target()\n\ndef caller_b(): target()\n\ndef caller_c(): target()\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.calls_edges, 3);

    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (c:Symbol)-[:CALLS]->(t:Symbol {name: 'target'}) RETURN c.name")
        .unwrap();
    let mut names: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "caller_a".to_string(),
            "caller_b".to_string(),
            "caller_c".to_string()
        ]
    );
}

#[test]
fn rust_macro_call_edge_created_when_macro_defined() {
    // Fixture: a Rust impl with a fn that calls another fn in the same file.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("lib.rs"),
        "fn helper() {}\n\nfn handler() {\n    helper();\n}\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(stats.calls_edges >= 1, "{stats:?}");
}

#[test]
fn reindex_is_idempotent_for_calls_edges() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def f(): pass\ndef g(): f()\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let s1 = build_index(&store, &repo).unwrap();
    let s2 = build_index(&store, &repo).unwrap();
    assert_eq!(s1.calls_edges, s2.calls_edges);
    assert_eq!(s1.calls_edges, 1);
}
