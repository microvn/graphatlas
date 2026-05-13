//! Story A — `seed_file_hint` MUST NOT silently fall back to global
//! search when the hinted file has no matching symbol.
//!
//! Driver: M3 minimal_context audit on regex (4/12 tasks recall=0)
//! revealed engine returning a wrong-crate file as seed when the
//! hinted file's indexed symbols don't include the requested name —
//! masking either a stale/wrong GT hint OR an indexer extraction gap.
//! See /mf-voices Round 4 (Codex + Claude Haiku consensus).
//!
//! Contract:
//!   for_symbol_in_file(sym, hint, budget):
//!     - hint matches a Symbol(name=sym, file=hint) → return that
//!     - hint doesn't match (file has no `sym` symbol) → return
//!       SymbolNotFound error (NOT a wrong-file fallback)
//!     - no hint set → existing global lookup behaviour preserved

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

/// Two files with same-named function: `fmt` in both `a.rs` and `b.rs`.
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
fn hint_miss_returns_symbol_not_found_not_fallback() {
    // Both files have `fmt`, but the hint points to a non-existent file.
    // Per /mf-voices consensus: silent fallback to a.rs or b.rs hides
    // the GT hint mismatch. Engine must surface SymbolNotFound instead.
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol_in_file("fmt", "does-not-exist.rs", 2000);
    let result = minimal_context(&store, &req);
    assert!(
        matches!(result, Err(ga_core::Error::SymbolNotFound { .. })),
        "hint miss must return SymbolNotFound; got {:?}",
        result.as_ref().err().map(|e| e.to_string())
    );
}

#[test]
fn hint_exact_match_resolves_correctly() {
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol_in_file("fmt", "a.rs", 2000);
    let resp = minimal_context(&store, &req).expect("must resolve at hinted file");

    let seed = resp
        .symbols
        .iter()
        .find(|s| s.symbol == "fmt")
        .expect("seed `fmt` returned");
    assert_eq!(seed.file, "a.rs");
}

#[test]
fn no_hint_preserves_global_lookup_regression() {
    // Without seed_file_hint, the existing global lookup must still
    // succeed — this is the regression guard. (Distinct from the hint
    // fix, which only changes hinted behaviour.)
    let tmp = fixture_two_fmts();
    let repo = tmp.path().join("repo");
    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol("fmt", 2000);
    let resp = minimal_context(&store, &req).expect("no-hint path still works");
    assert!(resp.symbols.iter().any(|s| s.symbol == "fmt"));
}
