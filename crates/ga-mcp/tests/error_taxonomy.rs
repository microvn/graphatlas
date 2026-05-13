//! Phase A review finding [H-1] — rmcp ServerHandler must preserve
//! ga_core::Error → JSON-RPC code mapping per Foundation-C5 + AS-023.
//!
//! Regression: ga-mcp/src/lib.rs:119 collapsed all Err variants to
//! McpError::internal_error (-32603). InvalidParams was meant to surface
//! as -32602 with suggestions (AS-014 qualified-seed-not-found path).

use ga_index::Store;
use ga_query::indexer::build_index;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::service::ServiceExt;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::duplex;

fn client_info() -> ClientInfo {
    let mut impl_info = Implementation::default();
    impl_info.name = "integration-test".into();
    impl_info.version = "0.0.0".into();
    let mut info = ClientInfo::default();
    info.capabilities = ClientCapabilities::default();
    info.client_info = impl_info;
    info
}

/// H-1 regression: qualified seed not found → JSON-RPC `-32602` (invalid
/// params), NOT `-32603` (internal error). Error message must carry
/// suggestions per AS-014.
#[tokio::test(flavor = "multi_thread")]
async fn invalid_params_maps_to_negative_32602_not_internal_error() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    // Small fixture so the Levenshtein suggestion has something to return.
    fs::write(
        repo.join("models.py"),
        b"class User:\n    def set_password(self):\n        pass\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let store = Arc::new(store);

    let (client_side, server_side) = duplex(64 * 1024);
    let server_store = store.clone();
    let server_handle = tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_side);
        let _ = ga_mcp::serve_with_store(r, w, server_store).await;
    });

    let (c_read, c_write) = tokio::io::split(client_side);
    let client = client_info()
        .serve((c_read, c_write))
        .await
        .expect("client init");

    // Qualified seed that does not resolve — triggers seed.rs
    // `Error::InvalidParams("Symbol not found: …")` path.
    let mut params = CallToolRequestParams::default();
    params.name = "ga_impact".into();
    params.arguments = Some(
        serde_json::json!({"symbol": "NonexistentClass.missing_method"})
            .as_object()
            .unwrap()
            .clone(),
    );

    let err = client
        .call_tool(params)
        .await
        .expect_err("tools/call must return JSON-RPC error for unresolvable qualified seed");

    // rmcp::ServiceError — extract inner ErrorData to assert the code.
    let err_str = format!("{err:?}");
    assert!(
        err_str.contains("-32602") || err_str.contains("InvalidParams"),
        "H-1: InvalidParams must surface as JSON-RPC -32602, NOT -32603. \
         Got: {err_str}"
    );
    assert!(
        !err_str.contains("-32603"),
        "H-1: error collapsed to -32603 Internal error — ga_core::Error \
         typed variant mapping broken. Got: {err_str}"
    );

    client.cancel().await.ok();
    let _ = tokio::time::timeout(Duration::from_secs(2), server_handle).await;
}
