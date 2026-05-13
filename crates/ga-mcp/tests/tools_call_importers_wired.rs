//! Tools S-003 cluster D — ga_importers MCP end-to-end.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
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

fn setup_store_with_files(files: &[(&str, &str)]) -> (TempDir, Arc<Store>) {
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
fn ga_importers_end_to_end_returns_direct_importers() {
    let (_tmp, store) = setup_store_with_files(&[
        ("utils/format.py", "def fmt(): pass\n"),
        ("a.py", "from utils.format import fmt\n"),
    ]);
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_importers".into(),
            arguments: json!({ "file": "utils/format.py" }),
        },
    )
    .expect("ga_importers should succeed");

    let payload = extract_json(&result);
    let importers = payload["importers"].as_array().expect("importers array");
    assert_eq!(importers.len(), 1);
    assert_eq!(importers[0]["path"], "a.py");
    assert_eq!(importers[0]["re_export"], false);
}

#[test]
fn ga_importers_payload_flags_re_export_and_via() {
    let (_tmp, store) = setup_store_with_files(&[
        ("bar.ts", "export function b() {}\n"),
        ("foo.ts", "export * from './bar';\n"),
        ("baz.ts", "import { b } from './foo';\n"),
    ]);
    let ctx = McpContext::new(store);

    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_importers".into(),
            arguments: json!({ "file": "bar.ts" }),
        },
    )
    .unwrap();

    let payload = extract_json(&result);
    let importers = payload["importers"].as_array().unwrap();
    let baz = importers
        .iter()
        .find(|i| i["path"] == "baz.ts")
        .unwrap_or_else(|| panic!("baz.ts expected as transitive importer: {importers:?}"));
    assert_eq!(baz["re_export"], true);
    assert_eq!(baz["via"], "foo.ts");
}

#[test]
fn ga_importers_missing_file_is_invalid() {
    let (_tmp, store) = setup_store_with_files(&[("a.py", "def f(): pass\n")]);
    let ctx = McpContext::new(store);

    let err = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_importers".into(),
            arguments: json!({}),
        },
    )
    .expect_err("missing file must error");
    let s = format!("{err}");
    assert!(s.contains("file") && s.contains("required"), "{s}");
}
