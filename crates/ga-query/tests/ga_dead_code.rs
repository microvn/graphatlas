//! S-003 ga_dead_code — entry-point-aware 0-in-degree detector.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-003):
//!   AS-008: Dead code list — happy path; entry points (routes, CLI,
//!     test fns, main, library public API) are filtered.
//!   AS-009: Scoped dead code — analysis bounded to a directory.
//!   AS-010: Library public API exclusion via `__all__` + pyproject
//!     `[project.scripts]`.
//!
//! Constraint (Tools-C4): entry-point detection MUST cover framework
//! routes, CLI commands, `main`, and library public API.

use ga_index::Store;
use ga_query::dead_code::{dead_code, DeadCodeRequest};
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (tmp, cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────
// AS-008 — Happy path: 0-in-degree symbols listed; entry points filtered.
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn lists_symbols_with_zero_callers() {
    let (_tmp, cache, repo) = setup();
    // `dead_helper` defined but never called → must surface.
    write(
        &repo.join("util.py"),
        "def dead_helper(x):\n    return x + 1\n\ndef live_helper(x):\n    return x - 1\n",
    );
    write(
        &repo.join("main.py"),
        "from util import live_helper\n\ndef driver():\n    return live_helper(1)\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        names.contains(&"dead_helper"),
        "dead_helper should appear in dead list; got {names:?}"
    );
    assert!(
        !names.contains(&"live_helper"),
        "live_helper has callers; must NOT be flagged dead"
    );
}

#[test]
fn dead_entry_carries_required_fields() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("u.py"), "def orphan():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let entry = resp
        .dead
        .iter()
        .find(|e| e.symbol == "orphan")
        .expect("orphan entry present");
    assert_eq!(entry.file, "u.py");
    assert!(
        entry.confidence >= 0.80,
        "AS-008: confidence ≥ 0.80; got {}",
        entry.confidence
    );
    assert!(
        !entry.entry_point_candidate,
        "non-entry-point symbol must have entry_point_candidate=false"
    );
    assert!(
        !entry.kind.is_empty(),
        "kind must be set (function/method/class/...)"
    );
}

#[test]
fn test_functions_excluded_as_entry_points() {
    // Tools-C4: tests are entry points (pytest discovery).
    let (_tmp, cache, repo) = setup();
    write(&repo.join("util.py"), "def helper():\n    return 0\n");
    write(
        &repo.join("tests/test_util.py"),
        "from util import helper\n\ndef test_helper_returns_zero():\n    assert helper() == 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"test_helper_returns_zero"),
        "test functions are entry points (pytest discovery) — must NOT be in dead list; got {names:?}"
    );
}

#[test]
fn main_function_excluded_as_entry_point() {
    // Tools-C4: `main` is an entry point even with no callers.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("cli.py"),
        "def main():\n    return 0\n\nif __name__ == '__main__':\n    main()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"main"),
        "`main` is an entry point per Tools-C4; got {names:?}"
    );
}

#[test]
fn django_route_handler_excluded_as_entry_point() {
    // Tools-C4: framework routes (Django path("...", view)) are entry points.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("views.py"),
        "def list_users(request):\n    return None\n\ndef detail_user(request, id):\n    return None\n",
    );
    write(
        &repo.join("urls.py"),
        "from django.urls import path\nfrom views import list_users, detail_user\n\nurlpatterns = [\n    path('users/', list_users),\n    path('users/<int:id>/', detail_user),\n]\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"list_users"),
        "Django route handler must NOT be flagged dead; got {names:?}"
    );
    assert!(
        !names.contains(&"detail_user"),
        "Django route handler must NOT be flagged dead; got {names:?}"
    );
}

#[test]
fn no_dead_code_returns_empty_list() {
    // All symbols connected — graceful empty.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "def a():\n    return 1\n\ndef main():\n    return a()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"a"),
        "`a` is called by main → not dead; got {names:?}"
    );
    assert!(
        !names.contains(&"main"),
        "`main` is entry point → not dead; got {names:?}"
    );
}

#[test]
fn empty_index_returns_index_not_ready() {
    // Tools-C1 — IndexNotReady when graph empty.
    let (_tmp, cache, repo) = setup();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    // Note: no build_index call.

    let res = dead_code(&store, &DeadCodeRequest::default());
    let err = res.expect_err("empty graph must Err with IndexNotReady");
    use ga_core::Error;
    assert!(
        matches!(err, Error::IndexNotReady { .. }),
        "expected IndexNotReady; got {err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-009 — Scoped analysis
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn scope_filter_restricts_to_directory() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("src/utils/lonely.py"),
        "def lonely():\n    return 0\n",
    );
    write(
        &repo.join("src/api/orphan.py"),
        "def api_orphan():\n    return 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = DeadCodeRequest {
        scope: Some("src/utils/".to_string()),
    };
    let resp = dead_code(&store, &req).expect("scoped dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        names.contains(&"lonely"),
        "src/utils/lonely.py:lonely must appear; got {names:?}"
    );
    assert!(
        !names.contains(&"api_orphan"),
        "src/api/* is out of scope; must be filtered; got {names:?}"
    );
}

#[test]
fn scope_filter_still_respects_entry_points() {
    // AS-009 §Then: "Entry points (test helpers in same dir) still excluded."
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("src/utils/helper.py"),
        "def util_helper():\n    return 0\n",
    );
    write(
        &repo.join("src/utils/tests/test_helper.py"),
        "from helper import util_helper\n\ndef test_util_helper():\n    assert util_helper() == 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = DeadCodeRequest {
        scope: Some("src/utils/".to_string()),
    };
    let resp = dead_code(&store, &req).expect("scoped dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"test_util_helper"),
        "scoped run still excludes test entry points; got {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-010 — Library public API exclusion
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn dunder_all_exports_excluded_as_entry_points() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("mylib/core.py"),
        "def public_api():\n    return 1\n\ndef private_helper():\n    return 0\n",
    );
    write(
        &repo.join("mylib/__init__.py"),
        "from .core import public_api\n\n__all__ = ['public_api']\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"public_api"),
        "`public_api` is in __all__ → library public API → not dead; got {names:?}"
    );
    // private_helper has no callers AND not in __all__ → dead.
    assert!(
        names.contains(&"private_helper"),
        "private_helper not in __all__ and uncalled → expected dead; got {names:?}"
    );
}

#[test]
fn project_scripts_entries_excluded_as_entry_points() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("mytool/cli.py"),
        "def cli_entry():\n    return 0\n",
    );
    write(
        &repo.join("pyproject.toml"),
        "[project]\nname = \"mytool\"\n\n[project.scripts]\nmytool = \"mytool.cli:cli_entry\"\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"cli_entry"),
        "[project.scripts] entry must be excluded per AS-010; got {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases — boundary, empty, special characters
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_scope_string_treated_as_no_scope() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def lonely():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = DeadCodeRequest {
        scope: Some(String::new()),
    };
    let resp = dead_code(&store, &req).expect("empty scope ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        names.contains(&"lonely"),
        "empty scope string must behave like no-scope; got {names:?}"
    );
}

#[test]
fn meta_reports_zero_caller_total_and_filtered_count() {
    // Surface the counts the spec implies (15 total, 8 reported, 7 filtered)
    // so the LLM agent can reason about the filter's coverage without
    // re-running with no entry-point filter.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("mod.py"),
        "def main():\n    return 0\n\ndef dead1():\n    return 0\n\ndef dead2():\n    return 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    assert_eq!(
        resp.meta.total_zero_caller, 3,
        "total_zero_caller counts all 0-in-degree symbols (main + dead1 + dead2)"
    );
    assert_eq!(
        resp.meta.entry_point_filtered, 1,
        "exactly 1 was filtered as entry point (main)"
    );
    assert_eq!(resp.dead.len(), 2, "2 surviving dead entries");
}
