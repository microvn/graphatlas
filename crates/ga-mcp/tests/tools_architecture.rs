//! S-005 ga_architecture MCP tool integration — descriptor + arg validation +
//! ctx-required dispatch (Tools-C1 + Tools-C5 read-only + Tools-C6 convention).

use ga_mcp::tools::{dispatch_tool_call, registered_tools};
use serde_json::json;

#[test]
fn ga_architecture_in_registered_tools_list() {
    let tools = registered_tools();
    assert!(
        tools.iter().any(|t| t.name == "ga_architecture"),
        "ga_architecture must be in tools/list output"
    );
}

#[test]
fn ga_architecture_descriptor_input_schema_accepts_max_modules() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_architecture")
        .expect("descriptor");
    let props = desc.input_schema.get("properties").expect("properties");
    let max_modules = props.get("max_modules").expect("max_modules property");
    assert_eq!(
        max_modules.get("type").and_then(|v| v.as_str()),
        Some("integer"),
        "max_modules must be typed as integer"
    );
    assert_eq!(
        max_modules.get("minimum").and_then(|v| v.as_i64()),
        Some(1),
        "max_modules minimum must be 1"
    );
}

#[test]
fn ga_architecture_descriptor_documents_tools_c6_convention() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_architecture")
        .expect("descriptor");
    let text = desc.description.to_lowercase();
    assert!(
        text.contains("convention_used") || text.contains("convention"),
        "Tools-C6: description must reference `meta.convention_used`; got: {}",
        desc.description
    );
}

#[test]
fn ga_architecture_ctxless_validates_then_returns_store_required() {
    let res = dispatch_tool_call("ga_architecture", &json!({}));
    let err = res.expect_err("ctxless must Err — store required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "Tools-C1: ctxless must indicate store/ctx required; got {msg}"
    );
}

#[test]
fn ga_architecture_ctxless_accepts_max_modules_int() {
    let res = dispatch_tool_call("ga_architecture", &json!({"max_modules": 30}));
    let err = res.expect_err("validation passes; ctx still required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "valid max_modules should pass validation; got {msg}"
    );
}

#[test]
fn ga_architecture_ctxless_rejects_non_object_args() {
    let res = dispatch_tool_call("ga_architecture", &json!("not an object"));
    let err = res.expect_err("non-object args must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("object") || msg.contains("invalid"),
        "non-object error message expected; got {msg}"
    );
}

#[test]
fn ga_architecture_ctxless_rejects_non_int_max_modules() {
    let res = dispatch_tool_call("ga_architecture", &json!({"max_modules": "thirty"}));
    let err = res.expect_err("non-int max_modules must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("max_modules") || msg.contains("integer") || msg.contains("invalid"),
        "type-error message expected; got {msg}"
    );
}

#[test]
fn ga_architecture_ctxless_rejects_zero_max_modules() {
    let res = dispatch_tool_call("ga_architecture", &json!({"max_modules": 0}));
    let err = res.expect_err("max_modules=0 must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("max_modules") || msg.contains("≥") || msg.contains("invalid"),
        "lower-bound error expected; got {msg}"
    );
}

#[test]
fn ga_architecture_ctxless_rejects_negative_max_modules() {
    let res = dispatch_tool_call("ga_architecture", &json!({"max_modules": -5}));
    let err = res.expect_err("negative max_modules must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("max_modules") || msg.contains("≥") || msg.contains("invalid"),
        "lower-bound error expected; got {msg}"
    );
}

#[test]
fn ga_architecture_descriptor_lists_in_correct_order_across_calls() {
    let a = registered_tools();
    let b = registered_tools();
    assert_eq!(
        a.iter().map(|t| &t.name).collect::<Vec<_>>(),
        b.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

#[test]
fn registered_tools_count_is_fifteen() {
    // Phase D close shipped 11; the centrality session added ga_hubs,
    // ga_bridges, ga_large_functions (14 total). v1.5 PR6 adds
    // ga_reindex (mutating tool, full-rebuild MVP) → 15. Bump again
    // whenever a new MCP tool is registered.
    let tools = registered_tools();
    assert_eq!(
        tools.len(),
        15,
        "expected 15 registered tools (14 read-only + ga_reindex); got {}",
        tools.len()
    );
}
