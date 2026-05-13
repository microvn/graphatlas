//! AS-017 — `tools/call` dispatch + error path.

use ga_mcp::error::to_jsonrpc_error;
use ga_mcp::handlers::handle_tools_call;
use ga_mcp::types::ToolsCallParams;
use serde_json::json;

#[test]
fn unknown_tool_returns_other_error() {
    let err = handle_tools_call(&ToolsCallParams {
        name: "ga_does_not_exist".into(),
        arguments: json!({}),
    })
    .expect_err("unknown tool must error");
    assert_eq!(err.jsonrpc_code(), -32099);
    let s = format!("{err}");
    assert!(s.contains("ga_does_not_exist"));
    assert!(
        s.contains("ga_callers") && s.contains("ga_callees") && s.contains("ga_importers"),
        "{s}"
    );
}

#[test]
fn ga_callers_missing_symbol_is_invalid() {
    let err = handle_tools_call(&ToolsCallParams {
        name: "ga_callers".into(),
        arguments: json!({ "file": "a.py" }),
    })
    .expect_err("missing symbol must error");
    // Missing required arg → Other error with actionable message.
    let s = format!("{err}");
    assert!(s.contains("symbol") && s.contains("required"), "err: {s}");
}

#[test]
fn ga_callers_without_ctx_returns_store_required_error() {
    // Cluster F landing: ctx-less handler can validate args but cannot run
    // the query. Callers must use `handle_tools_call_with_ctx`. Covered
    // end-to-end in tools_call_wired.rs.
    let err = handle_tools_call(&ToolsCallParams {
        name: "ga_callers".into(),
        arguments: json!({ "symbol": "foo" }),
    })
    .expect_err("ctx-less handler cannot run ga_callers");
    let s = format!("{err}").to_lowercase();
    assert!(
        s.contains("store") || s.contains("ctx"),
        "expected Store-context hint, got: {s}"
    );
}

#[test]
fn index_not_ready_end_to_end_maps_to_minus_32000_with_progress() {
    // AS-017 literal: response error {code: -32000, message, data:
    // {status: "indexing", progress: 0.4, eta_sec: 30}}. Here we construct
    // the error directly and confirm the mapper produces the spec shape.
    let err = ga_core::Error::IndexNotReady {
        status: "indexing".into(),
        progress: 0.4,
    };
    let jr = to_jsonrpc_error(&err);
    assert_eq!(jr.code, -32000);
    let data = jr.data.expect("data");
    assert_eq!(data["status"], "indexing");
    assert!(
        (data["progress"].as_f64().unwrap() - 0.4).abs() < 1e-6,
        "progress: {:?}",
        data["progress"]
    );
    // AS-017 requires `eta_sec` key exists (value may be null on first-run).
    assert!(data.get("eta_sec").is_some());
}

#[test]
fn tools_call_result_shape_matches_mcp() {
    // Happy-path shape check: if/when a real tool returns a result, JSON
    // serialization must produce {content: [...], isError?: bool}. Build
    // one synthetically to pin the shape before Tools S-001.
    use ga_mcp::types::{ContentBlock, ToolsCallResult};
    let r = ToolsCallResult {
        content: vec![ContentBlock::Json {
            json: json!({"ok": true}),
        }],
        is_error: false,
    };
    let v = serde_json::to_value(&r).unwrap();
    let content = v["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "json");
    assert_eq!(content[0]["json"]["ok"], true);
    // isError omitted when false (skip_serializing_if).
    assert!(v.get("isError").is_none() || v["isError"] == false);
}
