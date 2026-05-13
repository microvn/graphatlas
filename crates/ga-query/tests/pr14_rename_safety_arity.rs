//! v1.3 PR14 — `ga_rename_safety` arity-driven `param_count_changed` (S-003 AS-009 (b)).
//!
//! Spec: spec, AS-009.
//!
//! Then-clause: rename_safety with `new_arity` set returns `param_count_changed`
//! flag comparing against the target's DB-stored arity. UC consumers use this
//! to warn about API-breaking renames.
//!
//! Per-call-site arg-list filtering deferred — current CALLS edges don't store
//! arg count; documenting as future work in spec Known-Gap.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::rename_safety::{rename_safety, RenameSafetyRequest};
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

#[test]
fn new_arity_matches_existing_param_count_changed_false() {
    // Python `def foo(a, b)` (arity=2). Request new_arity=2 → no change.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(a, b):\n    return a + b\n\ndef caller():\n    return foo(1, 2)\n",
    );
    let (_t, store) = index_repo(repo.path());
    let report = rename_safety(
        &store,
        &RenameSafetyRequest {
            target: "foo".to_string(),
            replacement: "bar".to_string(),
            file_hint: None,
            new_arity: Some(2),
        },
    )
    .unwrap();
    assert!(
        !report.param_count_changed,
        "arity match should yield false"
    );
    assert_eq!(
        report.existing_arity,
        Some(2),
        "existing_arity must reflect DB column"
    );
}

#[test]
fn new_arity_differs_param_count_changed_true() {
    // `def foo(a, b)` (arity=2). Request new_arity=3 → diff.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(a, b):\n    return a + b\n\ndef caller():\n    return foo(1, 2)\n",
    );
    let (_t, store) = index_repo(repo.path());
    let report = rename_safety(
        &store,
        &RenameSafetyRequest {
            target: "foo".to_string(),
            replacement: "bar".to_string(),
            file_hint: None,
            new_arity: Some(3),
        },
    )
    .unwrap();
    assert!(
        report.param_count_changed,
        "new_arity=3 vs existing 2 must yield true"
    );
    assert_eq!(report.existing_arity, Some(2));
}

#[test]
fn no_new_arity_means_no_check() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(a, b):\n    return a + b\n\ndef caller():\n    return foo(1, 2)\n",
    );
    let (_t, store) = index_repo(repo.path());
    let report = rename_safety(
        &store,
        &RenameSafetyRequest {
            target: "foo".to_string(),
            replacement: "bar".to_string(),
            file_hint: None,
            new_arity: None,
        },
    )
    .unwrap();
    assert!(
        !report.param_count_changed,
        "no new_arity → no change reported"
    );
    // existing_arity may still be populated (informational) — but the flag
    // must not be raised without a comparison value.
}

#[test]
fn non_function_symbol_arity_minus_one_not_changed() {
    // Tools-C2: arity = -1 sentinel for non-function symbols. Request
    // new_arity should not raise param_count_changed against the sentinel.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "class Widget:\n    pass\n\ndef caller():\n    return Widget()\n",
    );
    let (_t, store) = index_repo(repo.path());
    let report = rename_safety(
        &store,
        &RenameSafetyRequest {
            target: "Widget".to_string(),
            replacement: "Gadget".to_string(),
            file_hint: None,
            new_arity: Some(2),
        },
    )
    .unwrap();
    assert!(
        !report.param_count_changed,
        "Tools-C2 sentinel: arity -1 means unknown, not 'differs'; param_count_changed must be false"
    );
}

#[test]
fn rust_def_arity_compared_correctly() {
    // Cross-lang verification — Rust `fn add(a: i32, b: i32) -> i32` (arity=2).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "fn add(a: i32, b: i32) -> i32 { a + b }\nfn caller() -> i32 { add(1, 2) }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let report = rename_safety(
        &store,
        &RenameSafetyRequest {
            target: "add".to_string(),
            replacement: "sum".to_string(),
            file_hint: None,
            new_arity: Some(3),
        },
    )
    .unwrap();
    assert!(
        report.param_count_changed,
        "Rust 2-arg fn vs new_arity=3 must report changed"
    );
    assert_eq!(report.existing_arity, Some(2));
}
