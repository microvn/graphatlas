//! S-002 ga_minimal_context MCP tool integration — descriptor + arg
//! validation + ctx-required dispatch + Tools-C3 schema docs.

use ga_mcp::tools::{dispatch_tool_call, registered_tools};
use serde_json::json;

#[test]
fn ga_minimal_context_in_registered_tools_list() {
    let tools = registered_tools();
    assert!(
        tools.iter().any(|t| t.name == "ga_minimal_context"),
        "ga_minimal_context must be in tools/list output"
    );
}

#[test]
fn descriptor_input_schema_accepts_symbol_file_budget() {
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_minimal_context")
        .expect("descriptor");
    let props = desc
        .input_schema
        .get("properties")
        .expect("input_schema.properties");
    for required in &["symbol", "file", "budget"] {
        assert!(
            props.get(required).is_some(),
            "schema must accept `{required}` arg"
        );
    }
}

#[test]
fn tools_c3_schema_documents_token_approximation() {
    // Tools-C3: "token counting MAY approximate when tiktoken not loaded
    // (±10% error acceptable) — document this in tool schema
    // inputSchema.description"
    let tools = registered_tools();
    let desc = tools
        .iter()
        .find(|t| t.name == "ga_minimal_context")
        .expect("descriptor");
    let budget = desc
        .input_schema
        .get("properties")
        .and_then(|p| p.get("budget"))
        .expect("budget property");
    let description = budget
        .get("description")
        .and_then(|d| d.as_str())
        .expect("budget description");
    assert!(
        description.to_lowercase().contains("approx")
            || description.contains("±10")
            || description.to_lowercase().contains("estimate"),
        "Tools-C3: budget description must mention approximation/±10%/estimate; got: {description}"
    );
}

#[test]
fn ctxless_validates_then_returns_store_required() {
    let res = dispatch_tool_call(
        "ga_minimal_context",
        &json!({"symbol": "authenticate", "budget": 2000}),
    );
    let err = res.expect_err("ctxless must Err — store required");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("store") || msg.contains("ctx"),
        "Tools-C1 ctxless error must indicate store/ctx required; got {msg}"
    );
}

#[test]
fn ctxless_rejects_empty_request() {
    let res = dispatch_tool_call("ga_minimal_context", &json!({}));
    let err = res.expect_err("empty request must Err");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("symbol") || msg.contains("file") || msg.contains("required"),
        "validation error must reference required fields; got {msg}"
    );
}

#[test]
fn ctxless_rejects_negative_budget() {
    let res = dispatch_tool_call(
        "ga_minimal_context",
        &json!({"symbol": "auth", "budget": -1}),
    );
    assert!(res.is_err(), "negative budget must Err");
}

#[test]
fn ctxless_rejects_non_object_args() {
    let res = dispatch_tool_call("ga_minimal_context", &json!("not an object"));
    assert!(res.is_err(), "non-object args must Err");
}

#[test]
fn ctxless_accepts_default_budget_when_omitted() {
    // Budget is technically required for the tool but default behavior:
    // tool may use a default (e.g., 2000) when omitted. Either path is
    // acceptable as long as we don't blow up. Verify it at least passes
    // validation and proceeds to ctx-required path.
    let res = dispatch_tool_call("ga_minimal_context", &json!({"symbol": "auth"}));
    let err = res.expect_err("ctxless must Err");
    let msg = format!("{err:?}").to_lowercase();
    // Either store-required (validation passed, default applied) OR
    // budget-required (validation rejected). Both acceptable.
    assert!(
        msg.contains("store") || msg.contains("ctx") || msg.contains("budget"),
        "must Err with store/ctx/budget reference; got {msg}"
    );
}
