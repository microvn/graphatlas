//! Tools S-006 cluster C9 — `diff` input wiring through `impact()`.
//! Unit tests for `extract_files_from_diff` live inline in `impact/diff.rs`;
//! these tests verify the end-to-end: diff text → union-of-files impact.

use ga_core::Error;
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

#[test]
fn diff_input_runs_impact_over_touched_files() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def a_fn(): pass\n\ndef a_caller():\n    a_fn()\n",
    );
    write(
        &repo.join("b.py"),
        "def b_fn(): pass\n\ndef b_caller():\n    b_fn()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let diff = "--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-x\n+y\n\
                --- a/b.py\n+++ b/b.py\n@@ -1 +1 @@\n-x\n+y\n";
    let resp = impact(
        &store,
        &ImpactRequest {
            diff: Some(diff.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&"a.py".to_string()), "{paths:?}");
    assert!(paths.contains(&"b.py".to_string()), "{paths:?}");
}

#[test]
fn diff_without_any_header_is_invalid_params() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def x(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // Non-empty but contains no diff header — malformed input.
    let err = impact(
        &store,
        &ImpactRequest {
            diff: Some("this is random text, not a diff".into()),
            ..Default::default()
        },
    )
    .expect_err("malformed diff must error");
    assert_eq!(err.jsonrpc_code(), -32602);
    assert!(
        format!("{err}").contains("diff"),
        "message should mention diff: {err}"
    );
    assert!(matches!(err, Error::InvalidParams(_)));
}

#[test]
fn diff_with_new_file_uses_plus_side_path() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("added.py"), "def added_fn(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let diff = "--- /dev/null\n+++ b/added.py\n@@\n+x\n";
    let resp = impact(
        &store,
        &ImpactRequest {
            diff: Some(diff.into()),
            ..Default::default()
        },
    )
    .unwrap();
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&"added.py".to_string()));
}

#[test]
fn diff_symbol_precedence_uses_symbol_not_diff() {
    // When both `symbol` and `diff` set, symbol wins (more specific).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def alpha(): pass\n\ndef alpha_caller():\n    alpha()\n",
    );
    write(&repo.join("unrelated.py"), "def beta(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            diff: Some("--- a/unrelated.py\n+++ b/unrelated.py\n@@\n".into()),
            ..Default::default()
        },
    )
    .unwrap();
    // impacted_files should come from symbol (a.py), not diff (unrelated.py).
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&"a.py".to_string()));
    assert!(!paths.contains(&"unrelated.py".to_string()), "{paths:?}");
}

#[test]
fn diff_single_file_happy_path() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let diff = "--- a/a.py\n+++ b/a.py\n@@ -1 +1 @@\n-x\n+y\n";
    let resp = impact(
        &store,
        &ImpactRequest {
            diff: Some(diff.into()),
            ..Default::default()
        },
    )
    .unwrap();
    // Break points should surface from the caller→target edge.
    assert!(
        !resp.break_points.is_empty(),
        "diff input must trigger BFS+break_points on touched files"
    );
}
