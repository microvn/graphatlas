//! v1.3 PR8 — DECORATES edge emission (S-006 AS-015 + AS-016 partial).
//!
//! Spec: spec, S-006.
//!
//! v1.3 scope (this PR):
//! - Python in-repo decorator → DECORATES(decorator_symbol → target_symbol).
//! - External decorators (stdlib `@functools.lru_cache`) drop silently per AS-015.
//! - Dotted names `@app.route` use last-segment lookup (`route`); deferred to
//!   PR-future when import-resolved name is needed.
//! - decorator_args extraction (`['/x', {methods: [GET]}]`) deferred to a
//!   later PR (per-lang AST work for arg JSON serialization).

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn decorates_pairs(store: &Store) -> Vec<(String, String)> {
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (d:Symbol)-[:DECORATES]->(t:Symbol) \
             RETURN d.name, t.name ORDER BY d.name, t.name",
        )
        .unwrap();
    let mut out = Vec::new();
    for row in rs {
        let mut it = row.into_iter();
        let d = match it.next() {
            Some(lbug::Value::String(s)) => s,
            _ => continue,
        };
        let t = match it.next() {
            Some(lbug::Value::String(s)) => s,
            _ => continue,
        };
        out.push((d, t));
    }
    out
}

#[test]
fn python_in_repo_decorator_emits_decorates_edge() {
    // Decorator in the same file → DECORATES edge present.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def my_decorator(fn):\n    return fn\n\
         \n\
         @my_decorator\n\
         def target_fn():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let pairs = decorates_pairs(&store);
    assert!(
        pairs.contains(&("my_decorator".to_string(), "target_fn".to_string())),
        "expected (my_decorator → target_fn) edge, got {pairs:?}"
    );
}

#[test]
fn python_external_decorator_drops_silently() {
    // AS-015: `@functools.lru_cache` is stdlib (not indexed) → no DECORATES edge.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "import functools\n\
         \n\
         @functools.lru_cache\n\
         def expensive():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let pairs = decorates_pairs(&store);
    assert!(
        !pairs.iter().any(|(_, t)| t == "expensive"),
        "external decorator must not emit DECORATES edge, got {pairs:?}"
    );
}

#[test]
fn python_dotted_decorator_resolves_last_segment_when_in_repo() {
    // `@helper.cached` → last-segment "cached". If helper module's `cached`
    // is in-repo, resolve via that name.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "helper.py", "def cached(fn):\n    return fn\n");
    write_file(
        repo.path(),
        "main.py",
        "import helper\n\
         \n\
         @helper.cached\n\
         def target():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let pairs = decorates_pairs(&store);
    assert!(
        pairs.iter().any(|(d, t)| d == "cached" && t == "target"),
        "expected (cached → target) via last-segment resolution, got {pairs:?}"
    );
}

#[test]
fn multiple_decorators_emit_one_edge_per_decorator() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def first(fn):\n    return fn\n\
         def second(fn):\n    return fn\n\
         \n\
         @first\n\
         @second\n\
         def stacked():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let pairs = decorates_pairs(&store);
    let stacked_decs: std::collections::HashSet<String> = pairs
        .iter()
        .filter(|(_, t)| t == "stacked")
        .map(|(d, _)| d.clone())
        .collect();
    assert!(
        stacked_decs.contains("first"),
        "expected `first` edge, got {pairs:?}"
    );
    assert!(
        stacked_decs.contains("second"),
        "expected `second` edge, got {pairs:?}"
    );
}

#[test]
fn no_decorator_no_edge() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "main.py", "def plain():\n    return 1\n");
    let (_t, store) = index_repo(repo.path());
    let pairs = decorates_pairs(&store);
    assert!(
        !pairs.iter().any(|(_, t)| t == "plain"),
        "plain function should have no DECORATES edge, got {pairs:?}"
    );
}
