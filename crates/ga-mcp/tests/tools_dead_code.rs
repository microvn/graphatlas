//! S-003 ga_dead_code MCP tool integration — descriptor + arg validation +
//! ctx-required dispatch + JSON-RPC error mapping (Tools-C1).

use ga_mcp::tools::{dispatch_tool_call, registered_tools};
use serde_json::json;

#[test]
fn ga_dead_code_in_registered_tools_list() {
    let tools = registered_tools();
    assert!(
        tools.iter().any(|t| t.name == "ga_dead_code"),
        "ga_dead_code must be in tools/list output"
    );
}

#[test]
fn ga_dead_code_descriptor_input_schema_accepts_scope() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_dead_code")
        .expect("ga_dead_code descriptor");
    let props = desc.input_schema.get("properties").expect("properties");
    assert!(
        props.get("scope").is_some(),
        "must accept optional `scope` arg"
    );
}

#[test]
fn ga_dead_code_ctxless_validates_then_returns_store_required() {
    // Tools-C1 — ctxless dispatch validates args first, then returns the
    // store-required error so the harness can route to ctx-call.
    let res = dispatch_tool_call("ga_dead_code", &json!({}));
    let err = res.expect_err("ctxless must Err — store required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "Tools-C1: ctxless error must indicate store/ctx required; got {msg}"
    );
}

#[test]
fn ga_dead_code_ctxless_accepts_scope_string() {
    let res = dispatch_tool_call("ga_dead_code", &json!({"scope": "src/utils/"}));
    let err = res.expect_err("validation passes; ctx still required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "valid scope should pass validation, fail at store-required boundary; got {msg}"
    );
}

#[test]
fn ga_dead_code_ctxless_rejects_non_object_args() {
    let res = dispatch_tool_call("ga_dead_code", &json!("not an object"));
    let err = res.expect_err("non-object args must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("object") || msg.contains("invalid"),
        "non-object error message expected; got {msg}"
    );
}

#[test]
fn ga_dead_code_ctxless_rejects_non_string_scope() {
    let res = dispatch_tool_call("ga_dead_code", &json!({"scope": 42}));
    let err = res.expect_err("non-string scope must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("scope") || msg.contains("string") || msg.contains("invalid"),
        "scope-type error expected; got {msg}"
    );
}

#[test]
fn ga_dead_code_descriptor_lists_in_correct_order_across_calls() {
    // Determinism guard.
    let a = registered_tools();
    let b = registered_tools();
    assert_eq!(
        a.iter().map(|t| &t.name).collect::<Vec<_>>(),
        b.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

#[test]
fn ga_dead_code_descriptor_documents_entry_point_classes() {
    // Tools-C4 — schema text should hint at the entry-point classes the
    // filter handles, so an LLM agent picking this tool understands the
    // assumptions baked into the result.
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_dead_code")
        .expect("ga_dead_code descriptor");
    let text = desc.description.to_lowercase();
    assert!(
        text.contains("entry point") || text.contains("entry-point"),
        "description should reference entry-point filtering"
    );
    assert!(
        text.contains("route") && (text.contains("main") || text.contains("__main__")),
        "description should mention routes + main; got: {}",
        desc.description
    );
}
