//! Tools S-006 cluster C0 — dispatcher wires ga_impact through to
//! `ga_query::impact`. Stub returns empty; real behavior lands in C1+.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use serde_json::json;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn empty_ctx() -> (TempDir, McpContext) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    (tmp, McpContext::new(Arc::new(store)))
}

fn extract_json(result: &ga_mcp::types::ToolsCallResult) -> &serde_json::Value {
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json,
        other => panic!("expected Json content block, got {other:?}"),
    }
}

#[test]
fn ga_impact_dispatcher_returns_empty_stub_response() {
    let (_tmp, ctx) = empty_ctx();

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: json!({ "symbol": "anything" }),
        },
    )
    .expect("ga_impact stub must not error");

    let payload = extract_json(&result);
    assert_eq!(payload["tool"], "ga_impact");
    assert!(payload["impacted_files"].as_array().unwrap().is_empty());
    assert!(payload["affected_tests"].as_array().unwrap().is_empty());
    assert!(payload["affected_routes"].as_array().unwrap().is_empty());
    assert!(payload["affected_configs"].as_array().unwrap().is_empty());
    assert!(payload["break_points"].as_array().unwrap().is_empty());
    assert_eq!(payload["risk"]["level"], "low");
}

#[test]
fn ga_impact_rejects_non_object_arguments() {
    let (_tmp, ctx) = empty_ctx();
    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: json!("not-an-object"),
        },
    )
    .expect_err("non-object args must error");
    let s = format!("{err}");
    assert!(
        s.contains("ga_impact") && s.contains("object"),
        "unexpected: {s}"
    );
}
