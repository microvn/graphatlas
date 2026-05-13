//! S-001 ga_risk MCP tool integration — descriptor + arg validation +
//! ctx-required dispatch + JSON-RPC error mapping (Tools-C1).

use ga_mcp::tools::{dispatch_tool_call, registered_tools};
use serde_json::json;

#[test]
fn ga_risk_in_registered_tools_list() {
    let tools = registered_tools();
    assert!(
        tools.iter().any(|t| t.name == "ga_risk"),
        "ga_risk must be in tools/list output"
    );
}

#[test]
fn ga_risk_descriptor_input_schema_has_symbol_and_changed_files() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_risk")
        .expect("ga_risk descriptor");
    let schema = &desc.input_schema;
    let props = schema.get("properties").expect("input_schema.properties");
    assert!(props.get("symbol").is_some(), "must accept `symbol` arg");
    assert!(
        props.get("changed_files").is_some(),
        "must accept `changed_files` arg"
    );
}

#[test]
fn ga_risk_unknown_tool_routes_to_error() {
    // Sanity: dispatcher rejects an obviously-wrong tool name so we know
    // the routing test below isn't a false positive.
    let res = dispatch_tool_call("ga_definitely_not_a_tool", &json!({}));
    assert!(res.is_err());
}

#[test]
fn ga_risk_ctxless_validates_then_returns_store_required() {
    // Tools-C1 contract: ctxless dispatch validates args first, then
    // returns the store-required error so the harness can route to ctx-call.
    let res = dispatch_tool_call("ga_risk", &json!({"symbol": "compute"}));
    let err = res.expect_err("ctxless must Err — store required");
    let msg = format!("{err:?}");
    assert!(
        msg.to_lowercase().contains("store") || msg.to_lowercase().contains("ctx"),
        "Tools-C1: ctxless error must indicate store/ctx required; got {msg}"
    );
}

#[test]
fn ga_risk_ctxless_rejects_empty_request() {
    // Tools-C1: validation runs before ctx-required short-circuit, so
    // a clearly-invalid request should produce a validation error
    // (not the store-required error).
    let res = dispatch_tool_call("ga_risk", &json!({}));
    let err = res.expect_err("empty request must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("symbol") || msg.contains("changed_files") || msg.contains("required"),
        "validation error must reference required fields; got {msg}"
    );
}

#[test]
fn ga_risk_ctxless_rejects_non_object_args() {
    let res = dispatch_tool_call("ga_risk", &json!("not an object"));
    let err = res.expect_err("non-object args must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("object") || msg.contains("invalid"),
        "non-object error message expected; got {msg}"
    );
}

#[test]
fn ga_risk_ctxless_rejects_invalid_changed_files_type() {
    // changed_files MUST be array-of-strings, not a single string.
    let res = dispatch_tool_call(
        "ga_risk",
        &json!({"changed_files": "single_string_not_array.py"}),
    );
    assert!(res.is_err(), "string `changed_files` must Err");
}

#[test]
fn ga_risk_descriptor_lists_in_correct_order_across_calls() {
    // Determinism guard — registered_tools is referentially stable.
    let a: Vec<String> = registered_tools().into_iter().map(|t| t.name).collect();
    let b: Vec<String> = registered_tools().into_iter().map(|t| t.name).collect();
    assert_eq!(a, b);
}
