//! KG-10 — affected_tests when seed is a test function.
//!
//! Regression: crates/ga-query/src/impact/affected_tests.rs:40 —
//! query `MATCH (prod)-[:TESTED_BY]->(test) WHERE prod.name = seed`
//! returns empty when seed is a test symbol (test symbols are never
//! the `prod` end of TESTED_BY edges per KG-1 emission rules).
//!
//! Fix: when seed lives in a test file, walk
//! (seed)-[:CALLS]->(prod)-[:TESTED_BY]->(sibling) and also include the
//! seed's own file.

use ga_index::Store;
use ga_query::impact::{impact, ImpactRequest};
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
fn affected_tests_includes_own_test_file_when_seed_is_test() {
    // Seed = `test_process_user` (in test_mod.py). It calls prod `process_user`.
    // Expected: affected_tests includes test_mod.py itself.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("mod.py"), "def process_user(u):\n    return u\n");
    write(
        &repo.join("test_mod.py"),
        "from mod import process_user\n\n\
         def test_process_user():\n    assert process_user(1) == 1\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ImpactRequest {
        symbol: Some("test_process_user".to_string()),
        ..Default::default()
    };
    let resp = impact(&store, &req).unwrap();

    let test_paths: Vec<String> = resp.affected_tests.iter().map(|t| t.path.clone()).collect();
    assert!(
        test_paths.iter().any(|p| p == "test_mod.py"),
        "expected seed's own test file in affected_tests, got {test_paths:?}"
    );
}

#[test]
fn affected_tests_includes_sibling_tests_when_seed_is_test() {
    // Two tests exercise the same prod symbol: test_a and test_b both call
    // process_user. Seed = test_a. Expected: affected_tests includes
    // test_a's file (self) AND test_b's file (sibling via prod).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("mod.py"), "def process_user(u):\n    return u\n");
    write(
        &repo.join("test_a.py"),
        "from mod import process_user\n\n\
         def test_a():\n    assert process_user(1) == 1\n",
    );
    write(
        &repo.join("test_b.py"),
        "from mod import process_user\n\n\
         def test_b():\n    assert process_user(2) == 2\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ImpactRequest {
        symbol: Some("test_a".to_string()),
        ..Default::default()
    };
    let resp = impact(&store, &req).unwrap();

    let test_paths: Vec<String> = resp.affected_tests.iter().map(|t| t.path.clone()).collect();
    assert!(
        test_paths.iter().any(|p| p == "test_b.py"),
        "expected sibling test file in affected_tests, got {test_paths:?}"
    );
}
