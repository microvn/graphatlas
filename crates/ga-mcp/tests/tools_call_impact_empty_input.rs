//! Tools S-006 cluster C1 — AS-015 through the MCP dispatcher.
//!
//! `ga_impact {changed_files: []}` must surface as JSON-RPC -32602 via
//! `to_jsonrpc_error` and carry a message that names the 3 input fields.

use ga_core::Error;
use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::error::to_jsonrpc_error;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::ToolsCallParams;
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

#[test]
fn ga_impact_empty_changed_files_maps_to_minus_32602() {
    let (_tmp, ctx) = empty_ctx();
    let err: Error = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: json!({ "changed_files": [] }),
        },
    )
    .expect_err("AS-015 must error");
    let jr = to_jsonrpc_error(&err);
    assert_eq!(jr.code, -32602);
    assert!(
        jr.message.contains("changed_files")
            && jr.message.contains("symbol")
            && jr.message.contains("diff"),
        "message must name all 3 inputs: {}",
        jr.message
    );
}

#[test]
fn ga_impact_fully_empty_request_maps_to_minus_32602() {
    let (_tmp, ctx) = empty_ctx();
    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: json!({}),
        },
    )
    .expect_err("no input fields must error");
    assert_eq!(to_jsonrpc_error(&err).code, -32602);
}
