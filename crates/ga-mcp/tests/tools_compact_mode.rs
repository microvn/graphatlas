//! P3.1 regression suite (2026-05-22) — compact mode default for
//! `ga_callers` / `ga_callees`. Aggregates per-call-site entries into one
//! entry per (caller_symbol, file) with a `call_sites` array + count.
//!
//! Rationale: LLM agent consuming the response can read "1 caller, 5 sites
//! [91, 117, 163, 177, 188]" in 1 entry instead of 5 repeated entries with
//! identical symbol/file/line. Less noise, easier reasoning.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! N2 + P3.1.

use ga_index::Store;
use ga_mcp::context::McpContext;
use ga_mcp::handlers::handle_tools_call_with_ctx;
use ga_mcp::types::{ContentBlock, ToolsCallParams};
use ga_query::indexer::build_index;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

fn setup_repeat_call_sites() -> (TempDir, Arc<Store>) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // `target` called 3 times from a single enclosing function. Pre-P3.1 this
    // returned 3 separate entries with the same caller_symbol/file. Compact
    // mode should dedup to 1 entry with call_sites: [3 lines].
    std::fs::write(
        repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n    target()\n    target()\n",
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

fn extract_text(result: &ga_mcp::types::ToolsCallResult) -> &str {
    match result.content.first() {
        Some(ContentBlock::Text { text }) => text.as_str(),
        other => panic!("expected Text content block, got {other:?}"),
    }
}

#[test]
fn callers_default_compact_dedups_per_caller_file() {
    // Regression: P3.1 — default returns 1 entry per (caller, file) with
    // call_sites array, not N entries per call site.
    let (_tmp, store) = setup_repeat_call_sites();
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
    let callers = payload["callers"].as_array().unwrap();
    assert_eq!(
        callers.len(),
        1,
        "default must dedup 3 sites → 1 caller entry: {payload}"
    );
    let entry = &callers[0];
    assert_eq!(entry["symbol"], "caller");
    assert_eq!(entry["file"], "m.py");
    let sites = entry["call_sites"].as_array().expect("call_sites array");
    assert_eq!(sites.len(), 3, "3 call sites in array: {entry}");
    assert_eq!(entry["call_site_count"], 3);
}

#[test]
fn callers_compact_preserves_distinct_callers() {
    // Two different callers, each calling target once. Compact must keep both
    // entries (dedup is per `(caller, file)`, not per caller across files).
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("m.py"),
        "def target(): pass\n\ndef caller_a():\n    target()\n\ndef caller_b():\n    target()\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let ctx = McpContext::new(Arc::new(store));
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().unwrap();
    assert_eq!(callers.len(), 2, "2 distinct callers preserved: {payload}");
    for c in callers {
        assert_eq!(c["call_site_count"], 1);
    }
}

#[test]
fn callers_verbosity_flat_opt_in_preserves_per_call_site() {
    // Opt-in `verbosity: "flat"` restores the pre-P3.1 per-call-site shape
    // for downstream that needs every line (refactor tools, bench, etc.).
    let (_tmp, store) = setup_repeat_call_sites();
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({ "symbol": "target", "verbosity": "flat" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callers = payload["callers"].as_array().unwrap();
    assert_eq!(
        callers.len(),
        3,
        "flat mode keeps 3 per-site entries: {payload}"
    );
}

#[test]
fn callees_default_compact_dedups() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("m.py"),
        "def helper(): pass\n\ndef driver():\n    helper()\n    helper()\n    helper()\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let ctx = McpContext::new(Arc::new(store));
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callees".into(),
            arguments: json!({ "symbol": "driver" }),
        },
    )
    .unwrap();
    let payload = extract_json(&result);
    let callees = payload["callees"].as_array().unwrap();
    assert_eq!(callees.len(), 1, "compact dedup callees: {payload}");
    let sites = callees[0]["call_sites"].as_array().expect("array");
    assert_eq!(sites.len(), 3);
}

#[test]
fn markdown_compact_shows_site_count() {
    let (_tmp, store) = setup_repeat_call_sites();
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
    // Compact markdown should mention call site count next to the entry.
    assert!(
        text.contains("3 sites") || text.contains("3 sites)") || text.contains("3 call sites"),
        "markdown must show site count: {text:?}"
    );
    assert!(text.contains("caller"));
}

#[test]
fn markdown_verbosity_flat_lists_each_site() {
    let (_tmp, store) = setup_repeat_call_sites();
    let ctx = McpContext::new(store);
    let result = handle_tools_call_with_ctx(
        &ctx,
        &ToolsCallParams {
            name: "ga_callers".into(),
            arguments: json!({
                "symbol": "target",
                "format": "markdown",
                "verbosity": "flat"
            }),
        },
    )
    .unwrap();
    let text = extract_text(&result);
    // Flat mode = 3 bullet lines (one per site).
    let bullets = text.lines().filter(|l| l.starts_with("- ")).count();
    assert_eq!(bullets, 3, "flat mode = 3 bullets: {text:?}");
}
