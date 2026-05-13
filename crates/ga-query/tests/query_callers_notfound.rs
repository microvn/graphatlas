//! Tools S-001 cluster D — AS-002 not-found path with Levenshtein top-3
//! suggestions. Spec binding: graphatlas-tools.md AS-002 + Tools-C9-d.

use ga_index::Store;
use ga_query::{callers, indexer::build_index};
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
fn not_found_returns_symbol_found_false_with_suggestions() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Symbols present: authenticate, auth_user, authorize.
    write(
        &repo.join("m.py"),
        "def authenticate(): pass\ndef auth_user(): pass\ndef authorize(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "autenticate", None).unwrap();
    assert!(resp.callers.is_empty(), "{:?}", resp.callers);
    assert!(!resp.meta.symbol_found);
    assert!(!resp.meta.suggestion.is_empty(), "expected suggestions");
    assert!(
        resp.meta.suggestion.contains(&"authenticate".to_string()),
        "{:?}",
        resp.meta.suggestion
    );
}

#[test]
fn found_without_callers_has_empty_suggestions() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def lonely(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "lonely", None).unwrap();
    assert!(resp.callers.is_empty());
    assert!(resp.meta.symbol_found, "symbol exists in graph");
    assert!(
        resp.meta.suggestion.is_empty(),
        "no suggestions when symbol found"
    );
}

#[test]
fn suggestions_ranked_by_edit_distance() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Distances from "cat": "cats"=1, "bat"=1, "cot"=1, "elephant"=~7.
    // Top suggestions should NOT include "elephant".
    write(
        &repo.join("m.py"),
        "def cats(): pass\ndef bat(): pass\ndef cot(): pass\ndef elephant(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "cat", None).unwrap();
    assert!(!resp.meta.symbol_found);
    assert!(
        !resp.meta.suggestion.contains(&"elephant".to_string()),
        "elephant is too far: {:?}",
        resp.meta.suggestion
    );
}

#[test]
fn suggestions_capped_at_three() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // 6 candidates, all 1-2 edits from "aaa".
    write(
        &repo.join("m.py"),
        "def aab(): pass\ndef aac(): pass\ndef aad(): pass\ndef aae(): pass\ndef aaf(): pass\ndef aag(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "aaa", None).unwrap();
    assert!(!resp.meta.symbol_found);
    assert!(
        resp.meta.suggestion.len() <= 3,
        "expected ≤3 suggestions, got {}: {:?}",
        resp.meta.suggestion.len(),
        resp.meta.suggestion
    );
}

#[test]
fn empty_graph_yields_empty_suggestions() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // No source files — graph has 0 symbols.
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "anything", None).unwrap();
    assert!(resp.callers.is_empty());
    assert!(!resp.meta.symbol_found);
    assert!(resp.meta.suggestion.is_empty());
}

#[test]
fn non_identifier_symbol_returns_empty_no_suggestions() {
    // Tools-C9-d: value-side identifier allowlist. Non-ident input must NOT
    // trigger a Cypher query or suggestion scan — both short-circuit to empty.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def target(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "tar'get", None).unwrap();
    assert!(resp.callers.is_empty());
    assert!(!resp.meta.symbol_found);
    assert!(resp.meta.suggestion.is_empty());
}
