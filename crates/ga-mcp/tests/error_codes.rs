//! AS-023 — ga_core::Error → JSON-RPC {code, message, data} mapping.

use ga_core::Error;
use ga_mcp::error::{to_jsonrpc_error, JsonRpcError};

#[test]
fn index_not_ready_maps_to_minus_32000() {
    let e = Error::IndexNotReady {
        status: "indexing".into(),
        progress: 0.4,
    };
    let jr: JsonRpcError = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32000);
    assert!(jr.message.to_lowercase().contains("index"));
    // AS-017 Then clause: data includes {status, progress, eta_sec}.
    let data = jr.data.expect("expected data");
    assert_eq!(data["status"], "indexing");
    assert!(data["progress"].is_number());
    // eta_sec may be null on first-run (unknown); presence of the key is the
    // contract. Value is best-effort.
    assert!(data.get("eta_sec").is_some());
}

#[test]
fn parse_error_maps_to_minus_32001() {
    let e = Error::ParseError {
        file: "a.py".into(),
        lang: "python".into(),
        err: "unexpected token".into(),
    };
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32001);
    assert!(jr.message.contains("a.py") || jr.message.to_lowercase().contains("parse"));
}

#[test]
fn config_corrupt_maps_to_minus_32002() {
    let e = Error::ConfigCorrupt {
        path: "/tmp/x".into(),
        reason: "bad json".into(),
    };
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32002);
}

#[test]
fn schema_mismatch_maps_to_minus_32003() {
    let e = Error::SchemaVersionMismatch {
        cache: 1,
        binary: 2,
    };
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32003);
    let data = jr.data.unwrap();
    assert_eq!(data["cache"], 1);
    assert_eq!(data["binary"], 2);
}

#[test]
fn io_error_maps_to_minus_32004() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
    let e: Error = io.into();
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32004);
}

#[test]
fn database_error_maps_to_minus_32005() {
    let e = Error::Database("connection refused".into());
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32005);
    assert!(jr.message.contains("connection refused"));
}

#[test]
fn other_error_maps_to_minus_32099() {
    let e = Error::Other(anyhow::anyhow!("whoops"));
    let jr = to_jsonrpc_error(&e);
    assert_eq!(jr.code, -32099);
}

#[test]
fn user_message_includes_remediation_hint() {
    // AS-023 Data: "User-facing error messages include remediation hints
    // (`run graphatlas doctor`)".
    let e = Error::ConfigCorrupt {
        path: "/tmp/x".into(),
        reason: "bad json".into(),
    };
    let jr = to_jsonrpc_error(&e);
    assert!(
        jr.message.to_lowercase().contains("doctor"),
        "config-corrupt error must suggest `graphatlas doctor`: {}",
        jr.message
    );
}
