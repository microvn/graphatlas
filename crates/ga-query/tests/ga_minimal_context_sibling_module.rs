//! Bug 4 — cross-module sibling discovery.
//!
//! When the seed lives in a Rust mod tree (`a/b/c.rs`) and other files
//! in the SAME directory call/reference the seed, those siblings should
//! surface as additional context. Currently engine only walks Caller /
//! Callee edges; siblings without an explicit edge don't appear.
//!
//! Driver: M3 minimal_context audit on axum found 4 of 12 tasks miss
//! sibling-module files (Router → method_routing.rs + path_router.rs;
//! strip_prefix → matched_path.rs + path/mod.rs; expand → with_position.rs).
//! See /mf-voices Round 4 ("name-only symbol identity" / cross-module
//! re-export class) — and the post-Round-4 axum audit that confirmed
//! the bulk of axum's remaining FAIL is this single bug.
//!
//! Contract: when the seed file lives in directory D, scan every other
//! `*.rs` file in D for occurrences of the seed symbol name (word
//! boundary) and emit up to MAX_SIBLINGS hits as a new `SiblingModule`
//! reason. Hits ranked by occurrence count (more references = more
//! related). Cross-crate is OUT of scope — this is "same Rust dir" only.

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

fn returned_files(resp: &ga_query::minimal_context::MinimalContextResponse) -> Vec<&str> {
    resp.symbols.iter().map(|s| s.file.as_str()).collect()
}

#[test]
fn rust_sibling_module_referencing_seed_is_surfaced() {
    // axum-c7d4af9b shape: seed Router lives in routing/mod.rs. Sibling
    // routing/method_routing.rs references it. Engine must surface that
    // sibling even when there's no explicit Caller/Callee edge to it
    // (e.g. when the reference is in a comment, type bound, or impl
    // block the indexer doesn't fully model).
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let dir = repo.join("routing");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("mod.rs"),
        "pub struct Router;\n\nimpl Router {\n    pub fn new() -> Self { Router }\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("method_routing.rs"),
        "// Module that mentions Router in a way the indexer may not link.\n\
         use crate::routing::Router;\n\n\
         pub fn make_router() -> Router { Router::new() }\n",
    )
    .unwrap();
    fs::write(
        dir.join("path_router.rs"),
        "// Sibling that also references Router.\n\
         pub struct PathRouter { inner: super::Router }\n",
    )
    .unwrap();

    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol("Router", 2000);
    let resp = minimal_context(&store, &req).expect("seed found");
    let files = returned_files(&resp);
    assert!(
        files
            .iter()
            .any(|f| f.ends_with("routing/method_routing.rs")),
        "sibling `routing/method_routing.rs` (references Router) not surfaced. \
         got files: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with("routing/path_router.rs")),
        "sibling `routing/path_router.rs` (references Router) not surfaced. \
         got files: {files:?}"
    );
}

#[test]
fn rust_siblings_ranked_by_occurrence_count() {
    // Hit cap: when a directory has many siblings referencing the seed,
    // pick the ones with the MOST occurrences first.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let dir = repo.join("mod");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("seed.rs"), "pub fn target_fn() -> i32 { 0 }\n").unwrap();
    // 4 siblings, each with different reference counts.
    fs::write(
        dir.join("low.rs"),
        "// only 1 reference\nuse crate::mod::seed::target_fn;\n",
    )
    .unwrap();
    fs::write(
        dir.join("medium.rs"),
        "// 3 references\nuse crate::mod::seed::target_fn;\nfn x() { target_fn(); target_fn(); }\n",
    )
    .unwrap();
    fs::write(
        dir.join("high.rs"),
        "// 5 references\n\
         use crate::mod::seed::target_fn;\n\
         fn x() { target_fn(); target_fn(); target_fn(); target_fn(); }\n",
    )
    .unwrap();
    fs::write(
        dir.join("none.rs"),
        "// 0 references — must NOT be surfaced as sibling\nfn unrelated() {}\n",
    )
    .unwrap();

    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol("target_fn", 2000);
    let resp = minimal_context(&store, &req).expect("seed found");
    let files = returned_files(&resp);
    assert!(
        !files.iter().any(|f| f.ends_with("mod/none.rs")),
        "file with 0 occurrences should NOT be a sibling. got: {files:?}"
    );
    // High-reference sibling should always be picked when cap kicks in.
    assert!(
        files.iter().any(|f| f.ends_with("mod/high.rs")),
        "highest-occurrence sibling not surfaced. got: {files:?}"
    );
}

#[test]
fn no_siblings_when_dir_has_only_seed() {
    // Seed alone in a dir: no siblings to surface, no crash.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("solo.rs"), "pub fn solo_fn() -> i32 { 0 }\n").unwrap();

    let (_cache, store) = open_store(&repo);
    build_index(&store, &repo).expect("build_index");

    let req = MinimalContextRequest::for_symbol("solo_fn", 2000);
    let resp = minimal_context(&store, &req).expect("must not crash");
    let files = returned_files(&resp);
    assert!(files.iter().any(|f| f.ends_with("solo.rs")));
}
