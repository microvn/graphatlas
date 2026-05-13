//! v1.3 PR13 — `ga_minimal_context` reads signature columns from DB (S-003 AS-009).
//!
//! Spec: spec, S-003 AS-009.
//!
//! When a symbol has populated `params != []` AND/OR `return_type != ''`,
//! the minimal_context envelope MUST compose its caller/callee signature
//! line from DB columns (no source-file re-read). Tools-C2 sentinel:
//! when params/return_type are empty, fall back to source-text read_snippet.
//!
//! PR13 closes the AS-009 (a) Then clause.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::minimal_context::{minimal_context, MinimalContextRequest};
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

#[test]
fn db_composed_signature_helper_handles_typed_python() {
    // Direct unit test on the helper. Tools-C2: empty type / default
    // emits a clean placeholder.
    let s = ga_query::minimal_context::compose_signature_from_db(
        &["pub".into(), "async".into()],
        "add",
        &[
            ("a".into(), "i32".into(), "".into()),
            ("b".into(), "i32".into(), "".into()),
        ],
        "i32",
    );
    assert!(
        s.contains("add"),
        "helper output must include name; got {s:?}"
    );
    assert!(
        s.contains("a: i32"),
        "helper output must include typed param; got {s:?}"
    );
    assert!(
        s.contains("b: i32"),
        "helper output must include typed param; got {s:?}"
    );
    assert!(
        s.contains("-> i32") || s.contains(": i32"),
        "helper output must include return type; got {s:?}"
    );
    assert!(
        s.contains("pub") && s.contains("async"),
        "helper output must include modifiers; got {s:?}"
    );
}

#[test]
fn db_composed_signature_helper_handles_unannotated_python() {
    // AS-008 sentinel: empty type + empty default → name only.
    let s = ga_query::minimal_context::compose_signature_from_db(
        &[],
        "foo",
        &[
            ("x".into(), "".into(), "".into()),
            ("y".into(), "".into(), "".into()),
            ("z".into(), "".into(), "10".into()),
        ],
        "",
    );
    assert!(s.contains("foo"));
    assert!(s.contains("x"));
    assert!(s.contains("y"));
    assert!(s.contains("z=10") || s.contains("z = 10"));
    assert!(
        !s.contains("->"),
        "unannotated return → no `->` arrow; got {s:?}"
    );
}

#[test]
fn db_composed_signature_helper_empty_params_returns_just_name() {
    // Tools-C2 sentinel: zero params → `name()`.
    let s = ga_query::minimal_context::compose_signature_from_db(&[], "nullary", &[], "");
    assert!(s.contains("nullary"));
    assert!(s.contains("()"));
}

#[test]
fn minimal_context_caller_signature_composed_from_db() {
    // Integration: build a fixture where a caller's signature is fully
    // populated in DB (Python typed). minimal_context for the seed should
    // include caller's signature line composed from DB cols.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def helper(a: int, b: int) -> int:\n    return a + b\n\
         \n\
         def caller(x: int, y: int) -> int:\n    return helper(x, y)\n",
    );
    let (_t, store) = index_repo(repo.path());
    // Force seed = helper. caller is its caller.
    let req = MinimalContextRequest::for_symbol("helper", 2000);
    let resp = minimal_context(&store, &req).unwrap();
    // One of the symbols entries must be the caller `caller`. Its snippet
    // (signature mode) should reflect DB composition: contains both param
    // names AND types.
    let names: Vec<String> = resp.symbols.iter().map(|s| s.symbol.clone()).collect();
    let caller_ctx = resp
        .symbols
        .iter()
        .find(|s| s.symbol == "caller")
        .unwrap_or_else(|| panic!("caller context should be present; got symbols={names:?}"));
    let snip = &caller_ctx.snippet;
    assert!(
        snip.contains("caller"),
        "caller snippet must contain name; got {snip:?}"
    );
    assert!(
        snip.contains("x") && snip.contains("y"),
        "caller snippet must contain param names from DB; got {snip:?}"
    );
    assert!(
        snip.contains("int"),
        "caller snippet must contain typed-param info from DB; got {snip:?}"
    );
}
