//! Bench P2-C1 — stateless MCP protocol core. Tests drive build + feed +
//! extract without needing a real child process.

use ga_bench::mcp::{McpCore, McpError};
use serde_json::{json, Value};

#[test]
fn build_initialize_matches_mcp_spec() {
    let mut core = McpCore::new();
    let bytes = core.build_initialize_bytes();
    let s = String::from_utf8(bytes).unwrap();
    // Line-delimited: initialize request + initialized notification.
    assert!(s.contains(r#""method":"initialize""#));
    assert!(s.contains(r#""method":"notifications/initialized""#));
    // protocolVersion present — servers ref 2024-11-05 (adapter TS spec).
    assert!(s.contains(r#""protocolVersion""#));
}

#[test]
fn build_tools_call_assigns_incrementing_ids() {
    let mut core = McpCore::new();
    let (id1, _) = core.build_tools_call_bytes("a", json!({}));
    let (id2, _) = core.build_tools_call_bytes("b", json!({}));
    assert!(id2 > id1);
}

#[test]
fn extract_response_matches_by_id() {
    let mut core = McpCore::new();
    let (id, _) = core.build_tools_call_bytes("test", json!({}));

    // Feed multiple responses; only the one matching id should be returned.
    let stream = format!(
        r#"{{"jsonrpc":"2.0","id":999,"result":{{"other":true}}}}
{{"jsonrpc":"2.0","id":{id},"result":{{"matched":true}}}}
"#
    );
    core.feed(&stream);
    let result = core.try_take_response(id).expect("matched response");
    assert_eq!(result["result"]["matched"], true);
}

#[test]
fn extract_response_tolerates_pretty_printed_multiline_json() {
    // GitNexus pretty-prints responses with newlines inside objects.
    let mut core = McpCore::new();
    let (id, _) = core.build_tools_call_bytes("x", json!({}));
    let stream = format!(
        "{{\n  \"jsonrpc\": \"2.0\",\n  \"id\": {id},\n  \"result\": {{\n    \"content\": \"ok\"\n  }}\n}}\n"
    );
    core.feed(&stream);
    let r = core.try_take_response(id).unwrap();
    assert_eq!(r["result"]["content"], "ok");
}

#[test]
fn extract_response_handles_nested_braces() {
    // Responses carrying JSON strings inside content must not fool the
    // balanced-brace scanner (MCP wraps inner JSON in {"type":"text","text":"..."}).
    let mut core = McpCore::new();
    let (id, _) = core.build_tools_call_bytes("x", json!({}));
    let stream = format!(
        r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":"{{\"nested\":true}}"}}]}}}}"#
    );
    core.feed(&stream);
    let r = core.try_take_response(id).unwrap();
    assert_eq!(r["result"]["content"][0]["type"], "text");
}

#[test]
fn extract_returns_none_when_stream_incomplete() {
    let mut core = McpCore::new();
    let (id, _) = core.build_tools_call_bytes("x", json!({}));
    // Half an object — no closing brace yet.
    core.feed(r#"{"jsonrpc":"2.0","id":1,"result":{"par"#);
    assert!(core.try_take_response(id).is_none());
}

#[test]
fn error_response_surfaces_as_mcp_error() {
    let mut core = McpCore::new();
    let (id, _) = core.build_tools_call_bytes("x", json!({}));
    let stream = format!(
        r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"Method not found"}}}}"#
    );
    core.feed(&stream);
    let r = core.try_take_response(id).unwrap();
    let err = McpCore::result_or_error(&r).unwrap_err();
    match err {
        McpError::Rpc { code, message } => {
            assert_eq!(code, -32601);
            assert!(message.contains("Method not found"));
        }
        other => panic!("expected Rpc error, got {other:?}"),
    }
}

#[test]
fn result_accessor_returns_result_value_on_success() {
    let v: Value = serde_json::from_str(r#"{"result":{"ok":1}}"#).unwrap();
    let r = McpCore::result_or_error(&v).unwrap();
    assert_eq!(r["ok"], 1);
}
