//! Tools S-006 cluster C0 — `ga_impact` appears in tools/list with a
//! permissive input schema covering the 3 input shapes.

use ga_mcp::handlers::handle_tools_list;

#[test]
fn tools_list_contains_ga_impact() {
    let result = handle_tools_list();
    let t = result
        .tools
        .iter()
        .find(|t| t.name == "ga_impact")
        .expect("ga_impact should be registered");
    assert!(!t.description.is_empty(), "description must be non-empty");
}

#[test]
fn ga_impact_input_schema_advertises_all_input_shapes() {
    let result = handle_tools_list();
    let t = result.tools.iter().find(|t| t.name == "ga_impact").unwrap();
    let schema = &t.input_schema;
    assert_eq!(schema["type"], "object");
    let props = &schema["properties"];
    assert_eq!(props["symbol"]["type"], "string");
    assert_eq!(props["file"]["type"], "string");
    assert_eq!(props["changed_files"]["type"], "array");
    assert_eq!(props["diff"]["type"], "string");
    assert_eq!(props["max_depth"]["type"], "integer");
}
