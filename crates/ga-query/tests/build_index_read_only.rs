//! Regression: build_index on a read-only store — docs/investigate/ga-anchord-watcher-reindex-brick-2026-06-20.md (action #2)
//!
//! build_index issues write DDL (DETACH DELETE + COPY). On a read-only store
//! (a peer process is the writer) lbug refuses the write, but only deep inside
//! the COPY phase — a messy late failure. The watcher fed read-only stores to
//! build_index and bricked itself on the resulting error.
//!
//! Durable invariant (action #2): build_index must REFUSE a read-only store up
//! front, before touching any DDL, with a clear writer-only message. This
//! pushes the "must be writer" invariant from implicit caller-discipline to an
//! enforced API contract so future callers can't reopen the same wound.

use ga_index::Store;
use ga_query::indexer::build_index;
use tempfile::TempDir;

fn fixture(tmp: &TempDir) -> std::path::PathBuf {
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        b"pub fn hello() -> &'static str { \"hi\" }\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    repo
}

#[test]
fn build_index_refuses_read_only_store_before_any_ddl() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = fixture(&tmp);

    // Open as writer, build once, then seal → flips the store to read-only
    // (drops the RW lbug handle, reopens read-only, releases the flock).
    let mut store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).expect("initial writer build must succeed");
    store.commit_in_place().unwrap();
    store.seal_for_serving().unwrap();
    assert!(store.is_read_only(), "sealed store must be read-only");

    // A second build on the read-only store must be refused up front with a
    // distinctive writer-only message (NOT the deep lbug COPY error).
    let err =
        build_index(&store, &repo).expect_err("build_index on a read-only store must be refused");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("read-only") && msg.contains("writable"),
        "must refuse with a clear writer-only message; got: {msg}"
    );
}
