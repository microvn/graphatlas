//! Tools-C1 — every MCP tool response carries
//! `meta.{query_time_ms, cache_hit, graph_version}` alongside per-tool meta.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn ctx() -> (TempDir, McpContext) {
    use ga_query::indexer::build_index;
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target():\n    pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let c = McpContext::new(Arc::new(store));
    (tmp, c)
}

fn call(ctx: &McpContext, name: &str, args: Value) -> Value {
    let r = handle_tools_call_with_ctx(
        ctx,
        &ToolsCallParams {
            name: name.into(),
            arguments: args,
        },
    )
    .expect(name);
    match r.content.first() {
        Some(ContentBlock::Json { json }) => json.clone(),
        _ => panic!("expected Json"),
    }
}

fn assert_common_meta(payload: &Value, tool: &str) {
    let meta = &payload["meta"];
    assert!(meta.is_object(), "{tool}: meta must be object: {payload}");
    let qt = meta["query_time_ms"].as_u64();
    assert!(qt.is_some(), "{tool}: query_time_ms missing in {meta}");
    assert!(meta["cache_hit"].is_boolean(), "{tool}: cache_hit missing");
    assert!(
        meta["graph_version"].as_u64().is_some(),
        "{tool}: graph_version missing"
    );
}

#[test]
fn ga_callers_emits_common_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_callers", json!({ "symbol": "target" }));
    assert_common_meta(&p, "ga_callers");
}

#[test]
fn ga_callees_emits_common_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_callees", json!({ "symbol": "caller" }));
    assert_common_meta(&p, "ga_callees");
}

#[test]
fn ga_importers_emits_common_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_importers", json!({ "file": "m.py" }));
    assert_common_meta(&p, "ga_importers");
}

#[test]
fn ga_symbols_emits_common_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_symbols", json!({ "pattern": "target" }));
    assert_common_meta(&p, "ga_symbols");
}

#[test]
fn ga_file_summary_emits_common_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_file_summary", json!({ "path": "m.py" }));
    assert_common_meta(&p, "ga_file_summary");
}

#[test]
fn ga_impact_emits_common_meta_alongside_impact_meta() {
    let (_tmp, c) = ctx();
    let p = call(&c, "ga_impact", json!({ "symbol": "target" }));
    assert_common_meta(&p, "ga_impact");
    // Existing impact-specific fields still present alongside common ones.
    assert!(p["meta"]["max_depth"].as_u64().is_some());
    assert!(p["meta"]["transitive_completeness"].as_u64().is_some());
}
