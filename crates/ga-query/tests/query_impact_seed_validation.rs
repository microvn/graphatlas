//! Tools S-006 cluster C1 — AS-015 input validation.
//!
//! ImpactRequest must have at least one of `symbol`, `changed_files` (non-empty),
//! or `diff` (non-empty). Otherwise the tool returns `Error::InvalidParams`
//! which the MCP layer maps to JSON-RPC -32602.

use ga_core::Error;
use ga_index::Store;
use ga_query::{impact, ImpactRequest};
use std::fs;
use tempfile::TempDir;

fn empty_store() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    (tmp, store)
}

fn assert_invalid_params(err: &Error) {
    assert_eq!(err.jsonrpc_code(), -32602, "expected -32602, got {err:?}");
    let msg = format!("{err}");
    assert!(
        msg.contains("changed_files") && msg.contains("symbol") && msg.contains("diff"),
        "message must enumerate the 3 inputs, got: {msg}"
    );
}

#[test]
fn impact_rejects_fully_empty_request() {
    let (_tmp, store) = empty_store();
    let err = impact(&store, &ImpactRequest::default())
        .expect_err("all-empty input must return InvalidParams");
    assert_invalid_params(&err);
}

#[test]
fn impact_rejects_empty_changed_files_array_as_015() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        changed_files: Some(Vec::new()),
        ..Default::default()
    };
    let err = impact(&store, &req).expect_err("AS-015: empty changed_files must error");
    assert_invalid_params(&err);
}

#[test]
fn impact_rejects_whitespace_only_symbol() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        symbol: Some("   ".into()),
        ..Default::default()
    };
    let err = impact(&store, &req).expect_err("whitespace symbol counts as absent");
    assert_invalid_params(&err);
}

#[test]
fn impact_rejects_empty_diff_string() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        diff: Some(String::new()),
        ..Default::default()
    };
    let err = impact(&store, &req).expect_err("empty diff counts as absent");
    assert_invalid_params(&err);
}

#[test]
fn impact_accepts_valid_symbol_returns_empty_response() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        symbol: Some("some_fn".into()),
        ..Default::default()
    };
    let resp = impact(&store, &req).expect("valid symbol must pass validation");
    // C1 does NOT implement BFS yet — expect still-empty response.
    assert!(resp.impacted_files.is_empty());
}

#[test]
fn impact_accepts_non_empty_changed_files() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        changed_files: Some(vec!["a.py".into()]),
        ..Default::default()
    };
    let resp = impact(&store, &req).expect("valid changed_files must pass");
    assert!(resp.impacted_files.is_empty());
}

#[test]
fn impact_accepts_non_empty_diff() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        diff: Some("--- a/x\n+++ b/x\n".into()),
        ..Default::default()
    };
    let resp = impact(&store, &req).expect("valid diff must pass");
    assert!(resp.impacted_files.is_empty());
}
