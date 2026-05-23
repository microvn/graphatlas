//! P1.5 / N1 regression suite (2026-05-22) — Markdown output format for
//! ga_callers / ga_callees / ga_impact / ga_symbols.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! N1 — GA's JSON-stringified content costs ~2× the tokens of CG's Markdown.
//! Opt-in `format: "markdown"` lets LLM-agent callers (Claude Code etc.)
//! request the cheaper format while programmatic callers keep the default
//! JSON shape.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use ga_query::indexer::build_index;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_store(repo_files: &[(&str, &str)]) -> (TempDir, Arc<Store>) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for (path, content) in repo_files {
        let p = repo.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    (tmp, Arc::new(store))
}

fn extract_text(result: &ga_mcp::types::ToolsCallResult) -> &str {
    match result.content.first() {
        Some(ContentBlock::Text { text }) => text.as_str(),
        other => panic!("expected Text content block, got {other:?}"),
    }
}

fn extract_json(result: &ga_mcp::types::ToolsCallResult) -> &Value {
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json,
        other => panic!("expected Json content block, got {other:?}"),
    }
}

// =============================================================================
// Default format = JSON (backward compat — existing 14 test files keep working)
// =============================================================================

#[test]
fn default_format_is_json_when_param_omitted() {
    // Regression: P1.5 — default must remain JSON so existing tests / programmatic
    // consumers don't break. Markdown is opt-in.
    let (_tmp, store) = setup_store(&[(
        "m.py",
        "def target(): pass\n\ndef caller():\n    target()\n",
    )]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    assert_eq!(payload["callers"][0]["symbol"], "caller");
}

// =============================================================================
// format=markdown → Text block, compact body
// =============================================================================

#[test]
fn callers_format_markdown_returns_text_block() {
    let (_tmp, store) = setup_store(&[(
        "m.py",
        "def target(): pass\n\ndef caller_a():\n    target()\n\ndef caller_b():\n    target()\n",
    )]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target", "format": "markdown" }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    assert!(
        text.starts_with("## Callers of target"),
        "markdown heading: {text:?}"
    );
    assert!(text.contains("caller_a"), "caller_a listed: {text:?}");
    assert!(text.contains("caller_b"), "caller_b listed: {text:?}");
    assert!(text.contains("m.py"), "file path included: {text:?}");
}

#[test]
fn callers_markdown_is_cheaper_than_json_for_same_query() {
    // N1 measurement check — Markdown body should be at least 30% smaller than
    // JSON body for the same caller set. Floor chosen so the test stays robust
    // across token-counting heuristic noise; observed ratio on gin Q4 was ~5×.
    let (_tmp, store) = setup_store(&[(
        "m.py",
        "def t(): pass\n\ndef c1(): t()\n\ndef c2(): t()\n\ndef c3(): t()\n\ndef c4(): t()\n\ndef c5(): t()\n",
    )]);
    let ctx = McpContext::new(store);
    let json_result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "t" }),
        },
    )
    .unwrap();
    let json_bytes = extract_json(&json_result).to_string().len();
    let md_result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "t", "format": "markdown" }),
        },
    )
    .unwrap();
    let md_bytes = extract_text(&md_result).len();
    assert!(
        md_bytes * 100 < json_bytes * 70,
        "Markdown ({md_bytes}b) must be ≤70% of JSON ({json_bytes}b)"
    );
}

#[test]
fn callees_format_markdown_lists_callees() {
    let (_tmp, store) = setup_store(&[(
        "m.py",
        "def helper(): pass\n\ndef driver():\n    helper()\n",
    )]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "symbol": "driver", "format": "markdown" }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    assert!(text.starts_with("## Callees of driver"), "{text:?}");
    assert!(text.contains("helper"), "{text:?}");
}

#[test]
fn impact_format_markdown_has_section_headings() {
    let (_tmp, store) = setup_store(&[
        ("m.py", "def seed(): pass\n\ndef caller(): seed()\n"),
        ("test_m.py", "def test_seed(): seed()\n"),
    ]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: json!({ "symbol": "seed", "format": "markdown" }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    assert!(text.starts_with("## Impact"), "heading: {text:?}");
    // Subsections — only render those that have entries.
    assert!(
        text.contains("### Impacted files") || text.contains("### Break points"),
        "expected subsection headings: {text:?}"
    );
}

#[test]
fn symbols_format_markdown_lists_results() {
    let (_tmp, store) = setup_store(&[("m.py", "def foo_bar(): pass\nclass FooBaz: pass\n")]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_symbols".into(),
            arguments: json!({ "pattern": "foo", "match": "fuzzy", "format": "markdown" }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    assert!(text.starts_with("## Search results"), "{text:?}");
}

// =============================================================================
// Disambiguation rendered as Markdown quote block
// =============================================================================

#[test]
fn callers_ambiguous_renders_markdown_disambiguation() {
    let (_tmp, store) = setup_store(&[
        ("a.py", "def Default(): pass\ndef caller_a(): Default()\n"),
        ("b.py", "def Default(): pass\ndef caller_b(): Default()\n"),
    ]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "Default", "format": "markdown" }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    assert!(
        text.contains("> Ambiguous") || text.contains("ambiguous"),
        "disambiguation header: {text:?}"
    );
    assert!(text.contains("a.py::Default"), "candidate a: {text:?}");
    assert!(text.contains("b.py::Default"), "candidate b: {text:?}");
}

#[test]
fn format_json_explicit_returns_json_block() {
    // Explicit format=json must work identically to omitted (backward compat).
    let (_tmp, store) = setup_store(&[("m.py", "def target(): pass\n\ndef caller(): target()\n")]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target", "format": "json" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    assert_eq!(payload["callers"][0]["symbol"], "caller");
}

#[test]
fn invalid_format_value_falls_back_to_json() {
    // Defensive — unknown format value should not error, just stick to JSON.
    let (_tmp, store) = setup_store(&[("m.py", "def target(): pass\n\ndef caller(): target()\n")]);
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target", "format": "xml" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    assert_eq!(payload["callers"][0]["symbol"], "caller");
}

// Suppress unused-import warning when only some tests reference Path.
#[allow(dead_code)]
fn _unused_path(_p: &Path) {}
