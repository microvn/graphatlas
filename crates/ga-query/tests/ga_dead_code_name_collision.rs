//! S-003 — `collect_zero_caller_symbols` name-collision fix.
//!
//! Per spec:
//! - AS-007.T1: same-name symbols in different files tracked independently.
//!   `a.py: def foo()` (no callers) + `b.py: def foo()` (called once) →
//!   only `a.py`'s `foo` is dead. The pre-fix implementation keyed
//!   `targeted` on bare `name`, so `b.py`'s call exempted `a.py`'s `foo`.
//! - AS-008.T1/T2: identity tuple `(name, file)` is the production
//!   contract; verified here by behaviour, not source-grep.
//! - AS-010: This file is the CI gate that enforces S-003 as a prerequisite
//!   for M3 bench S-005/S-006 (Hd-ast / Hrn-static). If S-003 is ever
//!   reverted, these tests fail in `cargo test --workspace` and CI goes
//!   red — that is the blockedBy enforcement teeth.

use ga_index::Store;
use ga_query::dead_code::{dead_code, DeadCodeRequest};
use ga_query::indexer::build_index;
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn as_007_t1_same_name_different_files_tracked_independently() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    // a.py — `foo` defined, never called anywhere → must be flagged dead.
    write(&repo.join("a.py"), "def foo():\n    return 1\n");
    // b.py — `foo` defined, called once locally → must NOT be flagged dead.
    write(
        &repo.join("b.py"),
        "def foo():\n    return 2\n\ndef driver():\n    return foo()\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");

    // Project: per-file (name, file) identity.
    let dead_pairs: Vec<(&str, &str)> = resp
        .dead
        .iter()
        .map(|e| (e.symbol.as_str(), e.file.as_str()))
        .collect();

    assert!(
        dead_pairs.contains(&("foo", "a.py")),
        "AS-007.T1: a.py::foo has zero callers anywhere — must be in dead list; got {dead_pairs:?}"
    );
    assert!(
        !dead_pairs.contains(&("foo", "b.py")),
        "AS-007.T1: b.py::foo has a local caller — must NOT be flagged dead. \
         Pre-fix bug: name-only `targeted` set let b.py's call exempt a.py's foo. \
         got dead_pairs: {dead_pairs:?}"
    );
}

#[test]
fn as_008_target_with_callers_in_one_file_does_not_exempt_homonym_in_another() {
    // Mirror AS-007 with three files to ensure the fix isn't a 2-file
    // happy-path coincidence.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("dead_a.py"),
        "def shared_name():\n    return 1\n",
    );
    write(
        &repo.join("dead_b.py"),
        "def shared_name():\n    return 2\n",
    );
    write(
        &repo.join("live.py"),
        "def shared_name():\n    return 3\n\ndef driver():\n    return shared_name()\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).unwrap();
    let dead_pairs: Vec<(&str, &str)> = resp
        .dead
        .iter()
        .map(|e| (e.symbol.as_str(), e.file.as_str()))
        .collect();

    for f in ["dead_a.py", "dead_b.py"] {
        assert!(
            dead_pairs.contains(&("shared_name", f)),
            "{f}::shared_name should be dead — only live.py's homonym has a caller. dead_pairs: {dead_pairs:?}"
        );
    }
    assert!(
        !dead_pairs.contains(&("shared_name", "live.py")),
        "live.py::shared_name has a local caller — must NOT be flagged dead. dead_pairs: {dead_pairs:?}"
    );
}
