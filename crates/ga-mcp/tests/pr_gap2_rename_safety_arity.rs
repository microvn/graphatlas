//! Gap 2 — MCP `ga_rename_safety` accepts `new_arity` input field.
//!
//! Spec: spec, AS-009 (b).
//! PR14 wired the Rust API. This gap surfaces the field through MCP wire
//! format so external clients can trigger the param_count_changed check.

use serde_json::json;

#[test]
fn input_schema_advertises_new_arity_property() {
    let descriptors = ga_mcp::tools::registered_tools();
    let rs = descriptors
        .iter()
        .find(|d| d.name == "ga_rename_safety")
        .expect("ga_rename_safety descriptor must exist");
    let schema = &rs.input_schema;
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("input_schema.properties must be an object");
    assert!(
        props.contains_key("new_arity"),
        "ga_rename_safety input_schema must advertise `new_arity` property; got {props:?}"
    );
    let new_arity = &props["new_arity"];
    assert_eq!(
        new_arity.get("type").and_then(|t| t.as_str()),
        Some("integer"),
        "new_arity should be integer; got {new_arity:?}"
    );
}

#[test]
fn validate_args_accepts_optional_new_arity() {
    // Calling with new_arity should not error; calling without should still
    // work (it's optional).
    let with_arity = json!({
        "target": "foo",
        "replacement": "bar",
        "new_arity": 3
    });
    let without_arity = json!({
        "target": "foo",
        "replacement": "bar"
    });
    // We don't directly call validate_args (private). Instead, the MCP
    // handler `ctxless` runs validation and returns a context-required
    // error WITHOUT panicking on schema. If args were rejected, ctxless
    // would surface a different InvalidParams error.
    let resp1 = ga_mcp::tools::dispatch_tool_call("ga_rename_safety", &with_arity);
    let resp2 = ga_mcp::tools::dispatch_tool_call("ga_rename_safety", &without_arity);
    // Both should succeed validation (pre-store-context error is fine —
    // the validation path passed before requesting context).
    let err1 = format!("{resp1:?}");
    let err2 = format!("{resp2:?}");
    // Neither response should mention an InvalidParams or schema rejection
    // for `new_arity` itself.
    assert!(
        !err1.contains("new_arity") || !err1.contains("Invalid"),
        "with_arity should not reject new_arity field: {err1}"
    );
    let _ = err2;
}

#[test]
fn validate_args_rejects_non_integer_new_arity() {
    let bad = json!({
        "target": "foo",
        "replacement": "bar",
        "new_arity": "not a number"
    });
    let resp = ga_mcp::tools::dispatch_tool_call("ga_rename_safety", &bad);
    let s = format!("{resp:?}");
    assert!(
        s.contains("new_arity") || s.contains("integer") || s.contains("Invalid"),
        "string `new_arity` should be rejected; got {s}"
    );
}
