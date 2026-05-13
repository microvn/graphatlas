//! Bench P2-C1 — McpChild integration against the `mcp-fake-echo` helper
//! binary (src/bin/mcp_fake_echo.rs). Exercises spawn + handshake +
//! tools_call + clean teardown without requiring a real MCP server.

use ga_bench::mcp::{McpChild, McpError};
use serde_json::json;
use std::time::Duration;

fn fake_server() -> &'static str {
    env!("CARGO_BIN_EXE_mcp_fake_echo")
}

#[test]
fn spawn_handshake_tools_call_happy_path() {
    let mut child = McpChild::spawn(&[fake_server()], Duration::from_secs(2))
        .expect("fake MCP server should spawn + handshake");
    let resp = child
        .tools_call("any_tool", json!({"x": 1}), Duration::from_secs(2))
        .expect("tools_call should succeed");
    // Fake echoes the method name that arrived.
    assert_eq!(resp["echoed"], "tools/call");
}

#[test]
fn spawn_nonexistent_command_reports_spawn_error() {
    let res = McpChild::spawn(
        &["/definitely/not/a/real/binary/___zzz___"],
        Duration::from_millis(500),
    );
    match res {
        Ok(_) => panic!("spawn must fail for bogus binary"),
        Err(McpError::Spawn(_)) => {}
        Err(other) => panic!("expected Spawn, got {other:?}"),
    }
}

#[test]
fn empty_command_rejected() {
    let res = McpChild::spawn(&[], Duration::from_millis(100));
    match res {
        Ok(_) => panic!("empty command must fail"),
        Err(McpError::Spawn(_)) => {}
        Err(other) => panic!("expected Spawn, got {other:?}"),
    }
}

#[test]
fn non_mcp_server_produces_clean_error_not_panic() {
    // `cat` reads stdin, echoes to stdout — it will echo our JSON-RPC
    // REQUESTS back. Those have matching ids but lack a `result` field, so
    // the client must surface either Malformed (echoed request parsed) or
    // Timeout (no response arrived in time). What it MUSTN'T do is panic
    // or hang. Either error variant proves the failure-mode is clean.
    let mut child = match McpChild::spawn(&["cat"], Duration::from_millis(300)) {
        Ok(c) => c,
        Err(_) => return, // handshake-layer failure — also acceptable
    };
    let res = child.tools_call("anything", json!({}), Duration::from_millis(200));
    match res {
        Ok(_) => panic!("cat can't synthesize a real MCP response"),
        Err(McpError::Timeout(_)) | Err(McpError::Malformed(_)) => {}
        Err(other) => panic!("expected Timeout or Malformed, got {other:?}"),
    }
}
