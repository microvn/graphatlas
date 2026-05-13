//! Tools S-006 cluster C11 — GaRetriever understands the `"impact"` UC so
//! the bench runner can score `ga_impact` quality against hand-curated GT.
//!
//! Scope-limited: we verify the dispatch wiring + return-shape invariants.
//! Full composite-≥0.80 measurement runs in the M2 gate against
//! `benches/uc-impact/ground-truth.json` (git-mined dataset).

use ga_bench::retriever::Retriever;
use ga_bench::retrievers::GaRetriever;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn ga_retriever_impact_uc_returns_impacted_paths() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    // target + caller — impact bfs surfaces both (same file → depth 0 only).
    write(
        &repo.join("auth.py"),
        "def check_password():\n    pass\n\n\
         def authenticate():\n    check_password()\n",
    );

    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    ret.setup(&repo).expect("setup");

    let result = ret
        .query("impact", &json!({ "symbol": "check_password" }))
        .expect("impact query");
    assert!(result.contains(&"auth.py".to_string()), "{result:?}");
}

#[test]
fn ga_retriever_impact_uc_accepts_changed_files_input() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(
        &repo.join("a.py"),
        "def a_fn():\n    pass\n\ndef a_caller():\n    a_fn()\n",
    );
    write(
        &repo.join("b.py"),
        "def b_fn():\n    pass\n\ndef b_caller():\n    b_fn()\n",
    );

    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    ret.setup(&repo).expect("setup");

    let result = ret
        .query("impact", &json!({ "changed_files": ["a.py", "b.py"] }))
        .expect("impact union");
    assert!(result.contains(&"a.py".to_string()));
    assert!(result.contains(&"b.py".to_string()));
}

#[test]
fn ga_retriever_impact_uc_empty_on_unknown_symbol() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def a_fn(): pass\n");

    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    ret.setup(&repo).expect("setup");

    let result = ret
        .query("impact", &json!({ "symbol": "nonexistent" }))
        .expect("impact query");
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn ga_retriever_impact_uc_missing_all_inputs_errors() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def a_fn(): pass\n");

    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    ret.setup(&repo).expect("setup");

    let err = ret
        .query("impact", &json!({}))
        .expect_err("AS-015: empty input must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("symbol") || msg.contains("changed_files") || msg.contains("diff"),
        "error should name required fields: {msg}"
    );
}
