//! Tools S-001 cluster A — indexer pipeline populates Store with File +
//! Symbol nodes + DEFINES edges.

use ga_index::Store;
use ga_query::indexer::{build_index, IndexStats};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn cache_and_repo(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

#[test]
fn empty_repo_indexes_to_zero_nodes() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats: IndexStats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.files, 0);
    assert_eq!(stats.symbols, 0);
    assert_eq!(stats.defines_edges, 0);
}

#[test]
fn single_python_file_writes_file_and_symbol_nodes() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(
        &repo.join("app.py"),
        "def greet():\n    pass\n\nclass User:\n    pass\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.files, 1);
    assert_eq!(stats.symbols, 2, "expected greet + User");

    let conn = store.connection().unwrap();

    // Verify File node exists.
    let mut found_file = false;
    let rs = conn
        .query("MATCH (f:File {path: 'app.py'}) RETURN f.lang")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::String(lang)) = row.into_iter().next() {
            assert_eq!(lang, "python");
            found_file = true;
        }
    }
    assert!(found_file);

    // Verify Symbol nodes exist with correct names.
    let rs = conn.query("MATCH (s:Symbol) RETURN s.name").unwrap();
    let names: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"greet".to_string()), "{names:?}");
    assert!(names.contains(&"User".to_string()), "{names:?}");
}

#[test]
fn multi_lang_repo_indexes_all_recognized_files() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(&repo.join("a.py"), "def f(): pass\n");
    write(&repo.join("b.rs"), "fn g() {}\n");
    write(&repo.join("c.go"), "package main\nfunc h() {}\n");
    write(&repo.join("README.md"), "# readme"); // filtered out

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.files, 3);
    assert!(stats.symbols >= 3);
}

#[test]
fn excluded_dirs_not_indexed() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(&repo.join("app.py"), "def a(): pass\n");
    write(&repo.join("node_modules/lib/index.js"), "function x(){}\n");
    write(&repo.join("target/debug/y.rs"), "fn z() {}\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.files, 1, "only app.py should survive exclude list");
}

#[test]
fn defines_edges_created_between_file_and_symbols() {
    // DEFINES: File → Symbol. One per symbol in a file.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(
        &repo.join("m.py"),
        "def a(): pass\ndef b(): pass\ndef c(): pass\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.symbols, 3);
    assert_eq!(stats.defines_edges, 3);

    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File)-[:DEFINES]->(s:Symbol) RETURN s.name")
        .unwrap();
    let names: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 3);
    for n in ["a", "b", "c"] {
        assert!(names.contains(&n.to_string()), "{names:?}");
    }
}

#[test]
fn duplicate_symbol_ids_are_deduped() {
    // Two identically-named functions in different files produce distinct
    // IDs via file-path qualification — both must land without collision.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(&repo.join("a.py"), "def dup(): pass\n");
    write(&repo.join("b.py"), "def dup(): pass\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.files, 2);
    assert_eq!(stats.symbols, 2);
}

#[test]
fn reindex_is_idempotent() {
    // Running build_index twice on the same repo must produce the same
    // counts (no duplicate PK errors). This is the foundation for reindex
    // (S-005) to work.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = cache_and_repo(&tmp);
    write(&repo.join("a.py"), "def f(): pass\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    let s1 = build_index(&store, &repo).unwrap();
    let s2 = build_index(&store, &repo).unwrap();
    assert_eq!(s1.files, s2.files);
    assert_eq!(s1.symbols, s2.symbols);
}
