//! Tools S-006 cluster C10 — MCP end-to-end coverage for `ga_impact`.
//!
//! Pins:
//! - AS-012: impact via MCP returns all 6 response fields + meta
//! - AS-014: route detection surfaces via MCP
//! - AS-016: depth + transitive_completeness meta
//! - Tools-C5: output caps + meta.truncated + meta.total_available
//! - Tools-C10: vendored-path meta.warning
//! - Adversarial: symbol allowlist + changed_files unsafe-char skip

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

fn ctx_for(cache: std::path::PathBuf, repo: std::path::PathBuf) -> McpContext {
    use ga_query::indexer::build_index;
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    McpContext::new(Arc::new(store))
}

fn call(ctx: &McpContext, args: Value) -> Value {
    let result = handle_tools_call_with_ctx(
        ctx,
        &ToolsCallParams {
            name: "ga_impact".into(),
            arguments: args,
        },
    )
    .expect("ga_impact dispatch");
    match result.content.first() {
        Some(ContentBlock::Json { json }) => json.clone(),
        other => panic!("expected Json block, got {other:?}"),
    }
}

#[test]
fn mcp_ga_impact_as012_returns_all_six_fields() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("auth.py"),
        "def check_password(): pass\n\ndef authenticate():\n    check_password()\n",
    );
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "check_password" }));
    for field in [
        "impacted_files",
        "affected_tests",
        "affected_routes",
        "affected_configs",
        "risk",
        "break_points",
        "meta",
    ] {
        assert!(payload.get(field).is_some(), "missing {field} in {payload}");
    }
    assert_eq!(payload["tool"], "ga_impact");
    // `check_password` has `authenticate` as caller — break points populated.
    assert!(
        !payload["break_points"].as_array().unwrap().is_empty(),
        "break_points must surface the authenticate→check_password edge"
    );
}

#[test]
fn mcp_ga_impact_as014_surfaces_gin_route() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handler.go"),
        "package main\n\nfunc CreateUser() {}\n",
    );
    write(
        &repo.join("routes.go"),
        "package main\n\nfunc setup(r *gin.Engine) {\n    r.POST(\"/api/users\", CreateUser)\n}\n",
    );
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "CreateUser" }));
    let routes = payload["affected_routes"].as_array().unwrap();
    assert_eq!(routes.len(), 1, "{routes:?}");
    assert_eq!(routes[0]["method"], "POST");
    assert_eq!(routes[0]["path"], "/api/users");
    assert_eq!(routes[0]["source_file"], "routes.go");
}

#[test]
fn mcp_ga_impact_as016_reports_max_depth_and_completeness() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef beta():\n    m = {'k': alpha}\n    return m\n",
    );
    write(
        &repo.join("c.py"),
        "from b import beta\n\ndef gamma():\n    m = {'k': beta}\n    return m\n",
    );
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "alpha", "max_depth": 2 }));
    assert_eq!(payload["meta"]["max_depth"], 2);
    assert_eq!(payload["meta"]["transitive_completeness"], 2);
    let paths: Vec<String> = payload["impacted_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    // Depth-2 chain reaches b.py (dep 1) and c.py (depth 2) via REFERENCES.
    assert!(paths.contains(&"b.py".to_string()), "{paths:?}");
    assert!(paths.contains(&"c.py".to_string()), "{paths:?}");
}

#[test]
fn mcp_ga_impact_output_cap_truncates_and_reports_total() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // 55 callers on same file — 56 break points after dedupe (well over cap).
    let mut src = String::from("def target(): pass\n\n");
    for i in 0..55 {
        src.push_str(&format!("def caller_{i}():\n    target()\n\n"));
    }
    write(&repo.join("m.py"), &src);
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "target" }));
    let bps = payload["break_points"].as_array().unwrap();
    assert!(bps.len() <= 50, "should cap at 50, got {}", bps.len());
    assert_eq!(payload["meta"]["truncated"]["break_points"], true);
    assert_eq!(
        payload["meta"]["total_available"]["break_points"], 55,
        "total reports real count pre-truncation"
    );
}

#[test]
fn mcp_ga_impact_vendor_warning_on_node_modules_path() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Seed + caller both live under node_modules → impacted_files will include it.
    // Use `third_party/` — in Tools-C10 warning list but NOT in the
    // walker's hard-excluded dirs (`node_modules` / `vendor` are skipped
    // by the walker so they can never surface in the first place).
    write(
        &repo.join("third_party/pkg/mod.py"),
        "def vendored():\n    pass\n\ndef v_caller():\n    vendored()\n",
    );
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "vendored" }));
    let warning = payload["meta"]["warning"].as_str();
    assert!(
        warning.is_some() && warning.unwrap().contains("vendored"),
        "expected vendored-path warning, got {:?}",
        warning
    );
}

#[test]
fn mcp_ga_impact_adversarial_symbol_quote_returns_safe_empty() {
    // Tools-C9-d — quote in symbol must short-circuit without Cypher.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def target(): pass\n");
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "symbol": "tar'get" }));
    assert!(payload["impacted_files"].as_array().unwrap().is_empty());
    assert!(payload["break_points"].as_array().unwrap().is_empty());
    assert_eq!(payload["risk"]["level"], "low");
}

#[test]
fn mcp_ga_impact_adversarial_changed_files_quote_path_skipped() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def a_fn(): pass\n");
    let ctx = ctx_for(cache, repo);

    let payload = call(&ctx, json!({ "changed_files": ["bad'path.py", "a.py"] }));
    // Good file still processed.
    let paths: Vec<String> = payload["impacted_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&"a.py".to_string()));
    // No trace of the evil path in the response.
    let s = payload.to_string();
    assert!(!s.contains("bad'path"), "evil path leaked: {s}");
}
