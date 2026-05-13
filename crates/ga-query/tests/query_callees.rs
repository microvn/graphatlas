//! Tools S-002 cluster A — AS-004 happy path for ga_callees.
//! Within-file resolved callees only (external marking = cluster B).

use ga_index::Store;
use ga_query::{callees, indexer::build_index};
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
fn direct_callees_returned() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def check_password(): pass\ndef get_user(): pass\ndef log_attempt(): pass\n\ndef authenticate():\n    check_password()\n    get_user()\n    log_attempt()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "authenticate", None).unwrap();
    assert_eq!(resp.callees.len(), 3, "{:?}", resp.callees);
    let mut names: Vec<String> = resp.callees.iter().map(|c| c.symbol.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "check_password".to_string(),
            "get_user".to_string(),
            "log_attempt".to_string()
        ]
    );
    for c in &resp.callees {
        assert_eq!(c.file, "m.py");
        assert_eq!(c.symbol_kind, "function");
    }
    assert!(resp.meta.symbol_found);
}

#[test]
fn empty_when_no_callees() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def leaf(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "leaf", None).unwrap();
    assert!(resp.callees.is_empty());
    assert!(resp.meta.symbol_found, "leaf exists");
}

#[test]
fn notfound_symbol_meta_symbol_found_false() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def authenticate(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "autenticate", None).unwrap();
    assert!(resp.callees.is_empty());
    assert!(!resp.meta.symbol_found);
    assert!(
        resp.meta.suggestion.contains(&"authenticate".to_string()),
        "{:?}",
        resp.meta.suggestion
    );
}

#[test]
fn multi_def_no_filter_yields_confidence_point_six() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    );
    write(
        &repo.join("b.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "shared", None).unwrap();
    assert_eq!(resp.callees.len(), 2);
    for c in &resp.callees {
        assert!(
            (c.confidence - 0.6).abs() < 1e-6,
            "expected 0.6, got {}",
            c.confidence
        );
    }
}

#[test]
fn file_filter_splits_exact_and_polymorphic() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    );
    write(
        &repo.join("b.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "shared", Some("a.py")).unwrap();
    let exact: Vec<_> = resp
        .callees
        .iter()
        .filter(|c| (c.confidence - 1.0).abs() < 1e-6)
        .collect();
    let poly: Vec<_> = resp
        .callees
        .iter()
        .filter(|c| (c.confidence - 0.6).abs() < 1e-6)
        .collect();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].file, "a.py");
    assert_eq!(poly.len(), 1);
    assert_eq!(poly[0].file, "b.py");
}

#[test]
fn non_identifier_symbol_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def auth(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "au'th", None).unwrap();
    assert!(resp.callees.is_empty());
    assert!(!resp.meta.symbol_found);
    assert!(resp.meta.suggestion.is_empty());
}
