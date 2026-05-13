//! Tools S-004 cluster C — ga_symbols MCP end-to-end.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::{handle_tools_call_with_ctx, handle_tools_list};
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use ga_query::indexer::build_index;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup_store(files: &[(&str, &str)]) -> (TempDir, Arc<Store>) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    for (path, content) in files {
        write(&repo.join(path), content);
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    (tmp, Arc::new(store))
}

fn extract_json(result: &ga_mcp::types::ToolsCallResult) -> &serde_json::Value {
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json,
        other => panic!("expected Json content block, got {other:?}"),
    }
}

#[test]
fn ga_symbols_registered_in_tools_list() {
    let result = handle_tools_list();
    let entry = result
        .tools
        .iter()
        .find(|t| t.name == "ga_symbols")
        .expect("ga_symbols must be registered");
    assert!(!entry.description.is_empty());
    let schema = &entry.input_schema;
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["pattern"]["type"], "string");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"pattern"), "{names:?}");
}

#[test]
fn ga_symbols_end_to_end_exact_match() {
    let (_tmp, store) = setup_store(&[("m.py", "def UserSerializer(): pass\n")]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_symbols".into(),
            arguments: json!({"pattern": "UserSerializer", "match": "exact"}),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let syms = payload["symbols"].as_array().unwrap();
    assert_eq!(syms.len(), 1);
    assert_eq!(syms[0]["name"], "UserSerializer");
    assert_eq!(payload["meta"]["truncated"], false);
}

#[test]
fn ga_symbols_fuzzy_mode_via_arg() {
    let (_tmp, store) = setup_store(&[("m.py", "def UserSerializer(): pass\n")]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_symbols".into(),
            arguments: json!({"pattern": "UsrSrlzr", "match": "fuzzy"}),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let syms = payload["symbols"].as_array().unwrap();
    assert!(!syms.is_empty());
    assert_eq!(syms[0]["name"], "UserSerializer");
}

#[test]
fn ga_symbols_missing_pattern_errors() {
    let (_tmp, store) = setup_store(&[("m.py", "def f(): pass\n")]);
    let ctx = McpContext::new(store);
    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_symbols".into(),
            arguments: json!({}),
        },
    )
    .expect_err("missing pattern must error");
    let s = format!("{err}");
    assert!(s.contains("pattern") && s.contains("required"), "{s}");
}
