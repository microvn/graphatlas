//! S-002 ga_minimal_context — composite token-budgeted retriever
//! integration tests.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-002):
//!   AS-005: Token-budgeted happy path.
//!   AS-006: File-level minimal context.
//!   AS-007: Budget too small — graceful partial.
//!   AS-016: Symbol not found — typed Err with Levenshtein suggestions.
//!
//! Priority order (AS-005 §Data): seed body > callers signatures >
//! callees signatures > imported types > test examples.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::minimal_context::{minimal_context, MinimalContextRequest, SymbolContextReason};
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
// AS-005 — Symbol-budget happy path
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn symbol_mode_returns_seed_callers_callees() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate(user, pw):\n    \"\"\"Verify user.\"\"\"\n    return check(pw)\n\ndef check(pw):\n    return pw == 'ok'\n",
    );
    write(
        &repo.join("login.py"),
        "from auth import authenticate\n\ndef login_view(req):\n    return authenticate(req.user, req.pw)\n",
    );
    write(
        &repo.join("session.py"),
        "from auth import authenticate\n\ndef session_start(u, p):\n    return authenticate(u, p)\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate", 2000);
    let resp = minimal_context(&store, &req).expect("symbol mode ok");

    let kinds: Vec<&SymbolContextReason> = resp.symbols.iter().map(|s| &s.reason).collect();
    assert!(
        kinds.iter().any(|r| **r == SymbolContextReason::Seed),
        "must include the seed; kinds: {:?}",
        kinds
    );
    let has_caller = kinds.iter().any(|r| **r == SymbolContextReason::Caller);
    let has_callee = kinds.iter().any(|r| **r == SymbolContextReason::Callee);
    assert!(
        has_caller || has_callee,
        "must include ≥1 caller or callee; kinds: {:?}",
        kinds
    );
}

#[test]
fn symbol_mode_seed_appears_first_in_priority_order() {
    // AS-005 §Data: priority (1) seed body, (2) callers signatures, (3) callees.
    // Seed must always be first when budget allows (the 0-th SymbolContext).
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate(user, pw):\n    return True\n",
    );
    write(
        &repo.join("login.py"),
        "from auth import authenticate\n\ndef login_view(req):\n    return authenticate(req.user, req.pw)\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate", 2000);
    let resp = minimal_context(&store, &req).unwrap();

    assert!(!resp.symbols.is_empty());
    assert_eq!(
        resp.symbols[0].reason,
        SymbolContextReason::Seed,
        "AS-005 priority: seed must be first; got {:?}",
        resp.symbols[0].reason
    );
}

#[test]
fn symbol_mode_token_estimate_within_budget() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate(user, pw):\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let budget = 2000u32;
    let req = MinimalContextRequest::for_symbol("authenticate", budget);
    let resp = minimal_context(&store, &req).unwrap();
    assert!(
        resp.token_estimate <= budget,
        "AS-005: token_estimate must be ≤ budget; got {} > {}",
        resp.token_estimate,
        budget
    );
}

#[test]
fn symbol_mode_budget_used_ratio_in_zero_one() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate():\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate", 2000);
    let resp = minimal_context(&store, &req).unwrap();
    assert!(
        resp.budget_used >= 0.0 && resp.budget_used <= 1.0,
        "budget_used ratio must be in [0,1]; got {}",
        resp.budget_used
    );
}

#[test]
fn symbol_mode_each_context_has_token_count() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate():\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate", 2000);
    let resp = minimal_context(&store, &req).unwrap();
    for ctx in &resp.symbols {
        // Empty snippet → 0 tokens is fine; non-empty must have non-zero.
        if !ctx.snippet.is_empty() {
            assert!(
                ctx.tokens > 0,
                "non-empty snippet must report >0 tokens; got snippet len {} tokens {}",
                ctx.snippet.len(),
                ctx.tokens
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AS-006 — File-level minimal context
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn file_mode_returns_exported_symbol_signatures() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "import hashlib\n\nclass Backend:\n    \"\"\"Auth backend.\"\"\"\n    def authenticate(self, u, p):\n        return self._check(u, p)\n    def _check(self, u, p):\n        return hashlib.sha256(p.encode())\n\ndef helper():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_file("auth.py", 1500);
    let resp = minimal_context(&store, &req).expect("file mode ok");
    assert!(
        !resp.symbols.is_empty(),
        "file mode must return ≥1 symbol context"
    );
    // Must reference at least one symbol from the file.
    let names: Vec<String> = resp.symbols.iter().map(|s| s.symbol.clone()).collect();
    assert!(
        names
            .iter()
            .any(|n| n == "Backend" || n == "authenticate" || n == "helper"),
        "must surface symbols from auth.py; got {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-007 — Budget too small — graceful partial (NOT error)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn budget_too_small_returns_partial_with_warning_not_error() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate_with_a_very_long_signature(user_argument: str, password_argument: str, session_token: str) -> bool:\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate_with_a_very_long_signature", 5);
    let resp = minimal_context(&store, &req).expect("AS-007: tiny budget MUST NOT error");
    assert!(
        resp.meta.truncated,
        "AS-007: tiny budget must set meta.truncated=true"
    );
    assert!(
        resp.meta.warning.is_some(),
        "AS-007: tiny budget must emit meta.warning string"
    );
    let warning = resp.meta.warning.as_deref().unwrap();
    assert!(
        warning.to_lowercase().contains("budget"),
        "AS-007 warning must mention budget; got: {warning}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-016 — Symbol not found — typed Err with Levenshtein suggestions
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_symbol_returns_invalid_params_with_suggestions() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def authenticate():\n    pass\n");
    write(&repo.join("b.py"), "def authorize():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("autenticate", 2000); // typo
    let result = minimal_context(&store, &req);
    let err = result.expect_err("AS-016 unknown symbol must Err");
    use ga_core::Error;
    match err {
        Error::SymbolNotFound { suggestions } => {
            assert!(
                !suggestions.is_empty(),
                "AS-016 §Data: suggestions array must be non-empty"
            );
            assert!(
                suggestions
                    .iter()
                    .any(|s| s == "authenticate" || s == "authorize"),
                "AS-016 suggestions must include nearest matches; got: {suggestions:?}"
            );
            assert!(
                suggestions.len() <= 3,
                "AS-016 §Then: suggestions capped at top-3 Levenshtein matches; got {} entries",
                suggestions.len()
            );
        }
        other => panic!("expected SymbolNotFound (structured); got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_request_neither_symbol_nor_file_errs() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def a():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::default();
    assert!(minimal_context(&store, &req).is_err());
}

#[test]
fn zero_budget_returns_empty_with_truncated_warning() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def a():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("a", 0);
    let resp = minimal_context(&store, &req).expect("zero budget is graceful");
    assert_eq!(resp.token_estimate, 0);
    assert!(resp.meta.truncated);
}

// ─────────────────────────────────────────────────────────────────────────
// AS-005 §Then "imported types" — priority (4) of priority order
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn symbol_mode_includes_imported_types_when_budget_allows() {
    // AS-005 §Then: response includes imported types alongside seed +
    // callers + callees. Implementation: TypeDep context block listing
    // file imports (pruned to those whose name appears in seed).
    let (_tmp, cache, repo) = setup();
    write(&repo.join("models.py"), "class User:\n    pass\n");
    write(
        &repo.join("auth.py"),
        "from models import User\n\ndef authenticate(u: User) -> bool:\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate", 5000);
    let resp = minimal_context(&store, &req).unwrap();
    let kinds: Vec<&SymbolContextReason> = resp.symbols.iter().map(|s| &s.reason).collect();
    assert!(
        kinds.iter().any(|r| **r == SymbolContextReason::TypeDep),
        "AS-005 priority (4): must include TypeDep imported-types block; got kinds {kinds:?}"
    );
    let imports_block = resp
        .symbols
        .iter()
        .find(|s| s.reason == SymbolContextReason::TypeDep)
        .expect("TypeDep block");
    assert!(
        imports_block.snippet.contains("models") || imports_block.snippet.contains("User"),
        "imports block should reference imported `models`/`User`; got: {}",
        imports_block.snippet
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-006 §Then "top-level imports + class docstrings"
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn file_mode_includes_top_level_imports() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "import hashlib\nimport os\n\nclass Backend:\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_file("auth.py", 2000);
    let resp = minimal_context(&store, &req).unwrap();
    let imports = resp
        .symbols
        .iter()
        .find(|s| s.reason == SymbolContextReason::TypeDep);
    assert!(
        imports.is_some(),
        "AS-006: file mode must include TypeDep imports block; got reasons {:?}",
        resp.symbols.iter().map(|s| &s.reason).collect::<Vec<_>>()
    );
    let block = imports.unwrap();
    assert!(
        block.snippet.contains("hashlib") || block.snippet.contains("os"),
        "imports block must reference top-level imports; got: {}",
        block.snippet
    );
}

#[test]
fn file_mode_class_includes_docstring() {
    // AS-006 §Then "class docstrings". For Python `class Foo:\n    """doc"""`
    // file mode should include the docstring alongside the class signature.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "class Backend:\n    \"\"\"Auth backend implementation.\"\"\"\n    def authenticate(self):\n        return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_file("auth.py", 2000);
    let resp = minimal_context(&store, &req).unwrap();
    let backend = resp
        .symbols
        .iter()
        .find(|s| s.symbol == "Backend")
        .expect("Backend class context");
    assert!(
        backend.snippet.contains("Auth backend implementation"),
        "AS-006: class snippet must include docstring; got: {}",
        backend.snippet
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-007 §Then exact warning string match (per spec literal)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn as007_warning_string_matches_spec_literal() {
    // Spec AS-007 §Then: meta.warning: "budget too small for full signature"
    // (exact string — LLM agents pattern-match on this text).
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def authenticate_with_a_very_long_signature(user: str, password: str, session: str) -> bool:\n    return True\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = MinimalContextRequest::for_symbol("authenticate_with_a_very_long_signature", 5);
    let resp = minimal_context(&store, &req).unwrap();
    let warning = resp.meta.warning.as_deref().expect("warning must be set");
    assert_eq!(
        warning, "budget too small for full signature",
        "AS-007 §Then: warning must be the EXACT spec string; got: {warning}"
    );
}

#[test]
fn budget_used_field_reflects_actual_consumption() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def a():\n    return 1\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let budget = 2000u32;
    let req = MinimalContextRequest::for_symbol("a", budget);
    let resp = minimal_context(&store, &req).unwrap();
    let expected = resp.token_estimate as f32 / budget as f32;
    assert!(
        (resp.budget_used - expected).abs() < 1e-3,
        "budget_used should match token_estimate/budget; got {}, expected {}",
        resp.budget_used,
        expected
    );
}
