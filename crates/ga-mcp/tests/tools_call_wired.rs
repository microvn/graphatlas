//! Tools S-001 cluster F — MCP dispatcher ↔ Store wiring end-to-end.
//!
//! Covers: `ga_callers` invoked through the MCP handler returns real data
//! pulled from an indexed Store (not the placeholder not-yet-wired error).

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
fn ga_callers_end_to_end_returns_real_callers() {
    let (_tmp, store) = setup_store("def target(): pass\n\ndef caller_a():\n    target()\n");
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target" }),
        },
    )
    .expect("ga_callers should succeed once Store is wired");

    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().expect("callers array");
    assert_eq!(callers.len(), 1, "{:?}", payload);
    assert_eq!(callers[0]["symbol"], "caller_a");
    assert_eq!(payload["meta"]["symbol_found"], true);
}

#[test]
fn ga_callers_notfound_surfaces_meta_symbol_found_false() {
    let (_tmp, store) = setup_store("def authenticate(): pass\ndef authorize(): pass\n");
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "autenticate" }),
        },
    )
    .expect("ga_callers should succeed even for unknown symbol");

    let payload = extract_json(&result);
    assert_eq!(payload["meta"]["symbol_found"], false);
    let suggestions = payload["meta"]["suggestion"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|s| s == "authenticate"),
        "expected 'authenticate' in suggestions: {suggestions:?}"
    );
}

#[test]
fn ga_callers_json_payload_includes_confidence() {
    let (_tmp, store) = setup_store("def target(): pass\n\ndef caller():\n    target()\n");
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target" }),
        },
    )
    .unwrap();

    let payload = extract_json(&result);
    let c = &payload["callers"][0];
    assert!(
        (c["confidence"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{c}"
    );
}
