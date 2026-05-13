//! AS-015 — MCP `initialize` handshake.
//!
//! Per spec: protocol version `"2025-11-25"`, server info `{name, version}`,
//! capabilities `{tools: {listChanged: false}}`.

use ga_mcp::handlers::{handle_initialize, InitializeParams};
use ga_mcp::types::InitializeResult;

#[test]
fn initialize_returns_spec_protocol_version() {
    let params = InitializeParams {
        protocol_version: "2025-11-25".to_string(),
        client_info: None,
    };
    let result: InitializeResult = handle_initialize(&params);
    assert_eq!(result.protocol_version, "2025-11-25");
    assert_eq!(result.protocol_version.len(), 10, "full YYYY-MM-DD");
}

#[test]
fn server_info_name_is_graphatlas() {
    let params = InitializeParams {
        protocol_version: "2025-11-25".to_string(),
        client_info: None,
    };
    let result = handle_initialize(&params);
    assert_eq!(result.server_info.name, "graphatlas");
    // Version pulled from CARGO_PKG_VERSION at compile time; must be non-empty
    // and start with a digit (semver).
    assert!(!result.server_info.version.is_empty());
    assert!(
        result
            .server_info
            .version
            .chars()
            .next()
            .unwrap()
            .is_ascii_digit(),
        "version should look like semver, got {:?}",
        result.server_info.version
    );
}

#[test]
fn capabilities_declare_tools_without_list_changed() {
    let params = InitializeParams {
        protocol_version: "2025-11-25".to_string(),
        client_info: None,
    };
    let result = handle_initialize(&params);
    assert!(result.capabilities.tools.is_some());
    let tools_cap = result.capabilities.tools.unwrap();
    assert!(
        !tools_cap.list_changed,
        "listChanged must be false per spec"
    );
}

#[test]
fn initialize_serializes_to_mcp_wire_shape() {
    // Sanity: the InitializeResult serializes to JSON keys that match the
    // MCP spec (camelCase): protocolVersion, serverInfo, capabilities.
    let params = InitializeParams {
        protocol_version: "2025-11-25".to_string(),
        client_info: None,
    };
    let result = handle_initialize(&params);
    let json = serde_json::to_value(&result).unwrap();
    assert!(json.get("protocolVersion").is_some(), "json: {json}");
    assert!(json.get("serverInfo").is_some(), "json: {json}");
    assert!(json.get("capabilities").is_some(), "json: {json}");

    let tools = json["capabilities"]["tools"].clone();
    assert!(tools.get("listChanged").is_some(), "tools: {tools}");
    assert_eq!(tools["listChanged"], false);
}

#[test]
fn initialize_ignores_client_protocol_mismatch_but_still_returns_server_version() {
    // Clients may send older / newer protocol versions during negotiation;
    // server always returns its own supported version. Mismatch handling is
    // up to client per MCP spec.
    let params = InitializeParams {
        protocol_version: "2099-01-01".to_string(),
        client_info: None,
    };
    let result = handle_initialize(&params);
    assert_eq!(result.protocol_version, "2025-11-25");
}
