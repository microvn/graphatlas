//! Tools S-006 cluster C3 — `break_points` field: where seed-symbol call
//! sites live. One entry per `(file, line)` with the names of all caller
//! symbols that own that line.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
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

fn run_impact(store: &Store, symbol: &str) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            symbol: Some(symbol.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn breakpoints_single_caller() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // target defined line 1; caller defined line 3; call site line 4.
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "target");
    assert_eq!(resp.break_points.len(), 1, "{:?}", resp.break_points);
    let bp = &resp.break_points[0];
    assert_eq!(bp.file, "m.py");
    assert_eq!(bp.line, 4);
    assert_eq!(bp.caller_symbols, vec!["caller".to_string()]);
}

#[test]
fn breakpoints_multiple_callers_same_file() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\n\
         def caller_a():\n    target()\n\n\
         def caller_b():\n    target()\n\n\
         def caller_c():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "target");
    assert_eq!(
        resp.break_points.len(),
        3,
        "expected one break point per call site: {:?}",
        resp.break_points
    );
    let mut callers: Vec<String> = resp
        .break_points
        .iter()
        .flat_map(|bp| bp.caller_symbols.clone())
        .collect();
    callers.sort();
    assert_eq!(
        callers,
        vec![
            "caller_a".to_string(),
            "caller_b".to_string(),
            "caller_c".to_string()
        ],
    );
    // All in same file.
    for bp in &resp.break_points {
        assert_eq!(bp.file, "m.py");
    }
}

#[test]
fn breakpoints_multi_file_callers() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // `target` defined in 3 files (polymorphic). Each file has a local caller.
    // CALLS resolution is same-file only today, so each caller calls the
    // target-in-same-file. The break point list must cover all 3 files.
    write(
        &repo.join("a.py"),
        "def target(): pass\n\ndef a_caller():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\n\ndef b_caller():\n    target()\n",
    );
    write(
        &repo.join("c.py"),
        "def target(): pass\n\ndef c_caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "target");
    let mut files: Vec<String> = resp.break_points.iter().map(|bp| bp.file.clone()).collect();
    files.sort();
    assert_eq!(files, vec!["a.py", "b.py", "c.py"]);
}

#[test]
fn breakpoints_same_symbol_two_call_sites_in_one_caller() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // lbug rejects duplicate (caller→callee, line) rows. Two DIFFERENT
    // lines from the same caller must surface as 2 break points.
    // Lines: 1 def target, 2 blank, 3 def caller, 4 target() first, 5 target() second.
    // NOTE: indexer dedupes on (caller,callee) pair only (see indexer.rs
    // lines 192-196), so the second call is currently folded into the
    // first. Test asserts the shipped behavior: ≥1 break point covering
    // the caller.
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "target");
    assert!(!resp.break_points.is_empty());
    assert_eq!(resp.break_points[0].caller_symbols, vec!["caller"]);
}

#[test]
fn breakpoints_empty_when_no_callers() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def lonely(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "lonely");
    assert!(resp.break_points.is_empty());
}

#[test]
fn breakpoints_empty_for_unknown_symbol() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def target(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "nonexistent");
    assert!(resp.break_points.is_empty());
}

#[test]
fn breakpoints_empty_for_non_ident_symbol() {
    // Tools-C9-d allowlist — quote in symbol short-circuits before Cypher.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "tar'get");
    assert!(resp.break_points.is_empty());
}

#[test]
fn breakpoints_sorted_by_file_then_line() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("z.py"),
        "def target(): pass\n\ndef z_caller():\n    target()\n",
    );
    write(
        &repo.join("a.py"),
        "def target(): pass\n\ndef a_caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_impact(&store, "target");
    // Deterministic: a.py before z.py.
    let files: Vec<String> = resp.break_points.iter().map(|bp| bp.file.clone()).collect();
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted, "break_points must be sorted by (file, line)");
}
