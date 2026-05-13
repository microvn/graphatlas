//! Tools S-005 cluster B — ga_file_summary MCP end-to-end.

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
fn ga_file_summary_registered_in_tools_list() {
    let result = handle_tools_list();
    let entry = result
        .tools
        .iter()
        .find(|t| t.name == "ga_file_summary")
        .expect("ga_file_summary must be registered");
    assert!(!entry.description.is_empty());
    let schema = &entry.input_schema;
    assert_eq!(schema["properties"]["path"]["type"], "string");
    let required = schema["required"].as_array().unwrap();
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"path"), "{names:?}");
}

#[test]
fn ga_file_summary_end_to_end() {
    let (_tmp, store) = setup_store(&[
        ("utils/format.py", "def fmt(): pass\n"),
        ("a.py", "from utils.format import fmt\ndef caller(): pass\n"),
    ]);
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_file_summary".into(),
            arguments: json!({"path": "a.py"}),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    assert_eq!(payload["path"], "a.py");
    let syms = payload["symbols"].as_array().unwrap();
    assert!(!syms.is_empty());
    let imports = payload["imports"].as_array().unwrap();
    assert!(imports.iter().any(|v| v == "utils/format.py"));
}

#[test]
fn ga_file_summary_missing_path_errors() {
    let (_tmp, store) = setup_store(&[("a.py", "def f(): pass\n")]);
    let ctx = McpContext::new(store);

    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_file_summary".into(),
            arguments: json!({}),
        },
    )
    .expect_err("missing path must error");
    let s = format!("{err}");
    assert!(s.contains("path") && s.contains("required"), "{s}");
}
