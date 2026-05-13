//! Regression tests: module-level TypePosition refs not flagged dead.
//!
//! Root cause: TypePosition refs with enclosing=None (impl Trait clauses,
//! module-level static/const type annotations) were dropped at indexer.rs:366,
//! causing types used only at module scope to be falsely flagged as dead.

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

#[test]
fn rust_impl_trait_not_flagged_dead() {
    let (_tmp, cache, repo) = setup();
    // WireData only appears in an `impl Trait` clause — no function ever calls it.
    write(&repo.join("types.rs"), "pub struct WireData;\n");
    write(
        &repo.join("consumer.rs"),
        "pub trait Marker {}\nimpl Marker for WireData {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let names: Vec<&str> = resp.dead.iter().map(|e| e.symbol.as_str()).collect();
    assert!(
        !names.contains(&"WireData"),
        "WireData used in impl Trait at module scope — must NOT be dead; got {names:?}"
    );
}
