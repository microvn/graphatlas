//! Story A — `MinimalContextRequest::for_symbol_in_file` API for
//! disambiguating generic seeds (`fmt`, `body`, `new`, etc.) when the
//! caller knows which file the symbol lives in.
//!
//! Driver: M3 minimal_context bench axum FAIL — 2/14 tasks have generic
//! seeds the engine cannot resolve correctly without a file hint.
//! See spec §Not in Scope.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::minimal_context::{minimal_context, MinimalContextRequest};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn open_store(fixture_dir: &PathBuf) -> (TempDir, Store) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_with_root(tmp.path(), fixture_dir).expect("open store");
    (tmp, store)
}

/// Build a tiny Rust fixture where two files BOTH define a function
/// named `fmt` (Display trait impl pattern). With no hint the engine may
/// resolve to either; with hint it must resolve to the requested file.
fn fixture_two_fmts() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("a.rs"),
        "pub struct A;\n\nimpl A {\n    pub fn fmt(&self) -> &str { \"A\" }\n}\n",
    )
    .unwrap();
    fs::write(
        repo.join("b.rs"),
        "pub struct B;\n\nimpl B {\n    pub fn fmt(&self) -> &str { \"B\" }\n}\n",
    )
    .unwrap();
    tmp
}

#[test]
fn for_symbol_in_file_disambiguates_to_requested_file() {
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol_in_file("fmt", "a.rs", 2000);
    let resp = minimal_context(&store, &req).expect("must resolve with hint");

    let seed = resp
        .symbols
        .iter()
        .find(|s| s.symbol == "fmt")
        .expect("seed `fmt` returned");
    assert_eq!(
        seed.file, "a.rs",
        "seed_file_hint=`a.rs` must resolve to a.rs, got `{}`",
        seed.file
    );
}

#[test]
fn for_symbol_in_file_returns_symbol_not_found_when_hint_file_has_no_match() {
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    // 2026-04-28 contract change: hint miss → SymbolNotFound, not silent
    // fallback. Old behaviour hid stale-hint + indexer-extraction bugs
    // under a wrong-file seed (regex audit, /mf-voices Round 4). This
    // test pins the new contract — see the dedicated test file
    // `ga_minimal_context_seed_hint_no_fallback.rs` for the full spec.
    let req = MinimalContextRequest::for_symbol_in_file("fmt", "does-not-exist.rs", 2000);
    let result = minimal_context(&store, &req);
    assert!(
        matches!(result, Err(ga_core::Error::SymbolNotFound { .. })),
        "hint miss must surface SymbolNotFound (no silent fallback); got {:?}",
        result.as_ref().err().map(|e| e.to_string())
    );
}

#[test]
fn for_symbol_without_hint_keeps_existing_behavior() {
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    // Regression: API extension must not break the no-hint path.
    let req = MinimalContextRequest::for_symbol("fmt", 2000);
    let resp = minimal_context(&store, &req).expect("no-hint path still works");
    assert!(resp.symbols.iter().any(|s| s.symbol == "fmt"));
}
