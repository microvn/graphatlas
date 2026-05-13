//! AS-016 — `tools/list` returns ga_callers with inputSchema validating
//! `symbol: string (required)`, `file: string (optional)`.

use ga_mcp::handlers::handle_tools_list;
use ga_mcp::types::ToolsListResult;

#[test]
fn tools_list_contains_ga_callers() {
    let result: ToolsListResult = handle_tools_list();
    assert!(!result.tools.is_empty(), "expected at least one tool");
    let callers = result
        .tools
        .iter()
        .find(|t| t.name == "ga_callers")
        .expect("ga_callers should be registered");
    assert!(
        !callers.description.is_empty(),
        "description must be non-empty"
    );
}

#[test]
fn ga_callers_input_schema_marks_symbol_required() {
    let result = handle_tools_list();
    let callers = result
        .tools
        .iter()
        .find(|t| t.name == "ga_callers")
        .unwrap();
    let schema = &callers.input_schema;

    assert_eq!(schema["type"], "object");
    let props = &schema["properties"];
    assert_eq!(props["symbol"]["type"], "string");
    assert_eq!(props["file"]["type"], "string");

    let required = schema["required"].as_array().expect("required array");
    let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        required_names.contains(&"symbol"),
        "required: {required_names:?}"
    );
    assert!(!required_names.contains(&"file"), "file must stay optional");
}

#[test]
fn tools_list_contains_ga_callees() {
    let result: ToolsListResult = handle_tools_list();
    let callees = result
        .tools
        .iter()
        .find(|t| t.name == "ga_callees")
        .expect("ga_callees should be registered");
    assert!(!callees.description.is_empty());
}

#[test]
fn ga_callees_input_schema_marks_symbol_required() {
    let result = handle_tools_list();
    let callees = result
        .tools
        .iter()
        .find(|t| t.name == "ga_callees")
        .unwrap();
    let schema = &callees.input_schema;
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["symbol"]["type"], "string");
    assert_eq!(schema["properties"]["file"]["type"], "string");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"symbol"), "{names:?}");
    assert!(!names.contains(&"file"));
}

#[test]
fn tools_list_contains_ga_importers() {
    let result: ToolsListResult = handle_tools_list();
    let importers = result
        .tools
        .iter()
        .find(|t| t.name == "ga_importers")
        .expect("ga_importers should be registered");
    assert!(!importers.description.is_empty());
}

#[test]
fn ga_importers_input_schema_marks_file_required() {
    let result = handle_tools_list();
    let importers = result
        .tools
        .iter()
        .find(|t| t.name == "ga_importers")
        .unwrap();
    let schema = &importers.input_schema;
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["file"]["type"], "string");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"file"), "{names:?}");
}

#[test]
fn tools_list_result_serializes_to_mcp_shape() {
    let result = handle_tools_list();
    let json = serde_json::to_value(&result).unwrap();
    // Top-level key is `tools` per MCP spec.
    let tools = json["tools"].as_array().expect("tools array");
    assert!(!tools.is_empty());
    let t0 = &tools[0];
    // Per MCP wire format: camelCase key `inputSchema`.
    assert!(t0.get("inputSchema").is_some(), "t0: {t0}");
    assert!(t0.get("name").is_some());
    assert!(t0.get("description").is_some());
}
