//! Tools S-002 cluster D — ga_callees MCP end-to-end via ctx-aware handler.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use ga_query::indexer::build_index;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_store(content: &str) -> (TempDir, Arc<Store>) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("m.py"), content).unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    (tmp, Arc::new(store))
}

fn extract_json(result: &ga_mcp::types::ToolsCallResult) -> &serde_json::Value {
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json,
        other => panic!("expected Json content block, got {other:?}"),
    }
}

#[test]
fn ga_callees_end_to_end_returns_real_callees() {
    let (_tmp, store) = setup_store("def helper(): pass\n\ndef caller():\n    helper()\n");
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "symbol": "caller" }),
        },
    )
    .expect("ga_callees should succeed");

    let payload = extract_json(&result);
    let callees = payload["callees"].as_array().expect("callees array");
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0]["symbol"], "helper");
    assert_eq!(payload["meta"]["symbol_found"], true);
}

#[test]
fn ga_callees_payload_flags_external_callee() {
    let (_tmp, store) = setup_store("def f():\n    sha256()\n");
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "symbol": "f" }),
        },
    )
    .unwrap();

    let payload = extract_json(&result);
    let callees = payload["callees"].as_array().unwrap();
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0]["symbol"], "sha256");
    assert_eq!(callees[0]["external"], true);
}

#[test]
fn ga_callees_missing_symbol_is_invalid() {
    let (_tmp, store) = setup_store("def f(): pass\n");
    let ctx = McpContext::new(store);

    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "file": "m.py" }),
        },
    )
    .expect_err("missing symbol must error");
    let s = format!("{err}");
    assert!(s.contains("symbol") && s.contains("required"), "{s}");
}
