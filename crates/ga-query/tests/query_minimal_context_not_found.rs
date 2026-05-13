//! S-002 AS-016 §Data — regression test (spec-mandated filename).
//!
//! Spec contract (graphatlas-v1.1-tools.md AS-016 §Data):
//!   "Regression test `tests/query_minimal_context_not_found.rs` asserts
//!    `-32602` with non-empty `suggestions` array."
//!
//! Spec contract (AS-016 §Setup):
//!   "Fixture with 3 real symbols (`foo`, `bar`, `baz`) pinned; assert
//!    `foo_not` → suggestions include `foo`."
//!
//! This file mirrors the spec literal so the regression is grep-able by
//! filename for future maintainers.

use ga_core::Error;
use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::minimal_context::{minimal_context, MinimalContextRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn foo_not_returns_symbol_not_found_with_suggestions_including_foo() {
    // AS-016 §Setup verbatim: pin foo/bar/baz fixture.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def foo():\n    return 1\n");
    write(&repo.join("b.py"), "def bar():\n    return 2\n");
    write(&repo.join("c.py"), "def baz():\n    return 3\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // AS-016 §When: query a typo (`foo_not`) — must NOT match any pinned
    // symbol, but should produce suggestions that include the closest
    // (`foo` — Levenshtein 4 from `foo_not`).
    let req = MinimalContextRequest::for_symbol("foo_not", 2000);
    let result = minimal_context(&store, &req);

    let err = result.expect_err("AS-016: typo `foo_not` must Err");
    match err {
        Error::SymbolNotFound { suggestions } => {
            // AS-016 §Data: "non-empty suggestions array".
            assert!(
                !suggestions.is_empty(),
                "AS-016 §Data: suggestions array must be non-empty"
            );
            // AS-016 §Setup: "suggestions include `foo`" — the closest match.
            assert!(
                suggestions.iter().any(|s| s == "foo"),
                "AS-016 §Setup: suggestions must include `foo` (closest to `foo_not`); got: {suggestions:?}"
            );
            // AS-016 §Then: "top-3 Levenshtein matches".
            assert!(
                suggestions.len() <= 3,
                "AS-016 §Then: suggestions capped at top-3; got {} entries",
                suggestions.len()
            );
        }
        other => panic!("AS-016 §Then: expected SymbolNotFound; got {other:?}"),
    }
}

#[test]
fn jsonrpc_code_for_symbol_not_found_is_minus_32602() {
    // AS-016 §Then: error code MUST be -32602 (JSON-RPC InvalidParams).
    let err = Error::SymbolNotFound {
        suggestions: vec!["foo".to_string()],
    };
    assert_eq!(
        err.jsonrpc_code(),
        -32602,
        "AS-016 §Then: code must be -32602; got {}",
        err.jsonrpc_code()
    );
}

#[test]
fn empty_index_returns_symbol_not_found_with_empty_suggestions() {
    // Defense-in-depth: if the graph is completely empty, suggestions
    // array is empty (not panic, not error variant change). LLM client
    // sees `-32602` + `data: {suggestions: []}` and knows the index is
    // unindexed.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    // Empty repo — no symbols at all.
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("anything", 2000);
    let err = minimal_context(&store, &req).expect_err("empty index must Err");
    match err {
        Error::SymbolNotFound { suggestions } => {
            assert!(
                suggestions.is_empty(),
                "empty index → empty suggestions; got {suggestions:?}"
            );
        }
        other => panic!("expected SymbolNotFound; got {other:?}"),
    }
}
