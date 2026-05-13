//! S-004 ga_rename_safety MCP tool integration — descriptor + arg validation +
//! ctx-required dispatch (Tools-C1 + Tools-C5 read-only contract).

use ga_mcp::tools::{dispatch_tool_call, registered_tools};
use serde_json::json;

#[test]
fn ga_rename_safety_in_registered_tools_list() {
    let tools = registered_tools();
    assert!(
        tools.iter().any(|t| t.name == "ga_rename_safety"),
        "ga_rename_safety must be in tools/list output"
    );
}

#[test]
fn ga_rename_safety_descriptor_input_schema_has_target_and_replacement() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_rename_safety")
        .expect("ga_rename_safety descriptor");
    let props = desc.input_schema.get("properties").expect("properties");
    assert!(props.get("target").is_some(), "must accept `target` arg");
    assert!(
        props.get("replacement").is_some(),
        "must accept `replacement` arg"
    );
    assert!(
        props.get("file").is_some(),
        "must accept optional `file` hint per Tools-C11"
    );
}

#[test]
fn ga_rename_safety_descriptor_marks_target_and_replacement_required() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_rename_safety")
        .expect("ga_rename_safety descriptor");
    let required = desc
        .input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required array present in inputSchema");
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        names.contains(&"target") && names.contains(&"replacement"),
        "target + replacement must be required; got {names:?}"
    );
}

#[test]
fn ga_rename_safety_ctxless_validates_then_returns_store_required() {
    let res = dispatch_tool_call(
        "ga_rename_safety",
        &json!({"target": "foo", "replacement": "bar"}),
    );
    let err = res.expect_err("ctxless must Err — store required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "Tools-C1: ctxless must indicate store/ctx required; got {msg}"
    );
}

#[test]
fn ga_rename_safety_ctxless_rejects_missing_target() {
    let res = dispatch_tool_call("ga_rename_safety", &json!({"replacement": "bar"}));
    let err = res.expect_err("missing target must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("target") || msg.contains("required"),
        "validation must reference missing target; got {msg}"
    );
}

#[test]
fn ga_rename_safety_ctxless_rejects_missing_replacement() {
    let res = dispatch_tool_call("ga_rename_safety", &json!({"target": "foo"}));
    let err = res.expect_err("missing replacement must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("replacement") || msg.contains("required"),
        "validation must reference missing replacement; got {msg}"
    );
}

#[test]
fn ga_rename_safety_ctxless_rejects_non_object_args() {
    let res = dispatch_tool_call("ga_rename_safety", &json!("not an object"));
    let err = res.expect_err("non-object args must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("object") || msg.contains("invalid"),
        "non-object error message expected; got {msg}"
    );
}

#[test]
fn ga_rename_safety_ctxless_rejects_non_string_target() {
    let res = dispatch_tool_call(
        "ga_rename_safety",
        &json!({"target": 42, "replacement": "bar"}),
    );
    let err = res.expect_err("non-string target must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("target") || msg.contains("string") || msg.contains("invalid"),
        "target-type error expected; got {msg}"
    );
}

#[test]
fn ga_rename_safety_descriptor_lists_in_correct_order_across_calls() {
    let a = registered_tools();
    let b = registered_tools();
    assert_eq!(
        a.iter().map(|t| &t.name).collect::<Vec<_>>(),
        b.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
}

#[test]
fn ga_rename_safety_descriptor_documents_read_only_contract() {
    // Tools-C5 — schema text should make it clear the tool does not write.
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_rename_safety")
        .expect("ga_rename_safety descriptor");
    let text = desc.description.to_lowercase();
    assert!(
        text.contains("read-only") || text.contains("agent invokes") || text.contains("decides"),
        "description should make read-only contract explicit; got: {}",
        desc.description
    );
}
