//! P1.3 regression suite (2026-05-22) — `include_uncertain` opt-in filter.
//!
//! When the caller passes a `file:` narrowing hint on a multi-def symbol,
//! GA used to surface BOTH the exact-match entries (confidence 1.0) AND
//! the polymorphic same-name entries from other files (confidence 0.6).
//! On tokio `block_on` (5 defs) that meant 2 conf-1.0 + 443 conf-0.6 = 445
//! entries / ~9k Markdown tokens or 17k JSON tokens per query — defeating
//! the point of the file hint.
//!
//! Fix: at the MCP wrapper, drop `confidence < 1.0` entries by default.
//! Caller opts back in via `include_uncertain: true` to restore the legacy
//! fan-out (kept for the rare downstream that needs blast radius).
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! Sprint 1 P1.3.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use ga_query::indexer::build_index;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

fn setup_multi_def() -> (TempDir, Arc<Store>) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("a.py"),
        "def target(): pass\ndef caller_a():\n    target()\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("b.py"),
        "def target(): pass\ndef caller_b():\n    target()\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    (tmp, Arc::new(store))
}

fn extract_json(result: &ga_mcp::types::ToolsCallResult) -> &Value {
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json,
        other => panic!("expected Json content block, got {other:?}"),
    }
}

#[test]
fn callers_file_hint_drops_polymorphic_by_default() {
    // Regression: P1.3 — file hint + multi-def used to leak conf 0.6 from
    // other defs. Default behavior must surface only conf 1.0 exact matches.
    let (_tmp, store) = setup_multi_def();
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target", "file": "a.py" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().unwrap();
    assert!(
        !callers.is_empty(),
        "exact caller (conf 1.0) must surface: {payload:?}"
    );
    for c in callers {
        let conf = c["confidence"].as_f64().unwrap();
        assert!(
            (conf - 1.0).abs() < 1e-6,
            "default must drop conf<1.0: got {conf} in {c:?}"
        );
    }
    // Meta should disclose how many uncertain entries were hidden.
    let hidden = payload["meta"]["hidden_uncertain_count"].as_u64();
    assert!(
        hidden.is_some() && hidden.unwrap() >= 1,
        "meta.hidden_uncertain_count must report dropped entries: {payload:?}"
    );
}

#[test]
fn callers_file_hint_include_uncertain_restores_polymorphic() {
    let (_tmp, store) = setup_multi_def();
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({
                "symbol": "target",
                "file": "a.py",
                "include_uncertain": true
            }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().unwrap();
    let exact_count = callers
        .iter()
        .filter(|c| (c["confidence"].as_f64().unwrap() - 1.0).abs() < 1e-6)
        .count();
    let poly_count = callers
        .iter()
        .filter(|c| (c["confidence"].as_f64().unwrap() - 0.6).abs() < 1e-6)
        .count();
    assert!(exact_count >= 1, "exact preserved: {payload:?}");
    assert!(
        poly_count >= 1,
        "polymorphic conf 0.6 must surface with opt-in: {payload:?}"
    );
}

#[test]
fn callees_file_hint_drops_polymorphic_by_default() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("a.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("b.py"),
        "def helper(): pass\ndef shared():\n    helper()\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let ctx = McpContext::new(Arc::new(store));
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "symbol": "shared", "file": "a.py" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callees = payload["callees"].as_array().unwrap();
    assert!(!callees.is_empty());
    for c in callees {
        let conf = c["confidence"].as_f64().unwrap();
        assert!((conf - 1.0).abs() < 1e-6, "got {conf} in {c:?}");
    }
}

#[test]
fn single_def_unaffected_by_p1_3_filter() {
    // Single-def: every entry is conf 1.0 already → filter is a no-op.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("m.py"),
        "def lonely(): pass\n\ndef c1(): lonely()\n\ndef c2(): lonely()\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let ctx = McpContext::new(Arc::new(store));
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "lonely" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().unwrap();
    assert_eq!(callers.len(), 2, "{payload:?}");
    // meta.hidden_uncertain_count present but 0 — informational.
    let hidden = payload["meta"]["hidden_uncertain_count"]
        .as_u64()
        .unwrap_or(0);
    assert_eq!(hidden, 0);
}

#[test]
fn markdown_path_also_filters_polymorphic_by_default() {
    // P1.3 must apply to the markdown render too — drop conf < 1.0 entries.
    let (_tmp, store) = setup_multi_def();
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({
                "symbol": "target",
                "file": "a.py",
                "format": "markdown"
            }),
        },
    )
    .unwrap();
    let text = match result.content.first() {
        Some(ContentBlock::Text { text }) => text.as_str(),
        other => panic!("expected Text, got {other:?}"),
    };
    // No `[conf=0.6]` annotations expected in default markdown mode.
    assert!(
        !text.contains("[conf=0.6]"),
        "default markdown must drop conf<1.0 entries: {text:?}"
    );
    assert!(
        text.contains("caller_a"),
        "exact caller preserved: {text:?}"
    );
}
