//! Regression: watcher self-bricks on reindex race — docs/investigate/ga-anchord-watcher-reindex-brick-2026-06-20.md (action #1)
//!
//! Each MCP process runs its own L1 watcher. On a repo with multiple concurrent
//! MCP servers (e.g. anchord), a file edit makes every watcher fire reindex.
//! The race loser re-attaches read-only. Pre-fix, the watcher's rebuild closure
//! lacked the `AttachedReadOnly` early-return guard that the MCP tool path has,
//! so it called build_index on the read-only store → lbug refused the write →
//! closure errored → rebuild_via left `store_cell = None` → the server lost its
//! graph.db handle ("bricked"); the next tool call's `ctx.store()` then panicked.
//!
//! This test reproduces the race with an external flock holder (mirrors
//! ga-index `multi_process_lock.rs`). After a reindex that LOSES the race, the
//! context's store cell must remain populated (server keeps serving read-only),
//! NOT be bricked.

use ga_index::Store;
use ga_mcp::context::McpContext;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tempfile::TempDir;

/// Resolve the ga-index `ga_index_lock_holder` helper (built into the
/// workspace target by a full `cargo test` run). Mirrors the resolver in
/// `crates/ga-index/tests/multi_process_lock.rs`.
fn helper_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")); // crates/ga-mcp
    let candidates = [
        manifest_dir.join("../../target/debug/ga_index_lock_holder"),
        manifest_dir.join("../../target/release/ga_index_lock_holder"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.canonicalize().expect("canonicalize helper path");
        }
    }
    panic!(
        "ga_index_lock_holder not found in target/{{debug,release}}/. \
         Run `cargo build -p ga-index --bin ga_index_lock_holder` first."
    );
}

fn spawn_exclusive_holder(cache_root: &Path, repo_root: &Path, secs: u64) -> Child {
    let mut child = Command::new(helper_path())
        .arg("--cache-root")
        .arg(cache_root)
        .arg("--repo-root")
        .arg(repo_root)
        .arg("--hold")
        .arg("exclusive")
        .arg("--secs")
        .arg(secs.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn lock holder");
    // Block until the helper signals READY (flock acquired).
    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read READY");
    assert_eq!(line.trim_end(), "READY", "helper must signal READY");
    child
}

fn fixture(tmp: &TempDir) -> PathBuf {
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), b"pub fn hello() {}\n").unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        b"[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    repo
}

#[test]
fn watcher_reindex_losing_race_does_not_brick_store_cell() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = fixture(&tmp);

    // Build + commit, then seal so this process holds NO flock (mirrors a
    // post-boot serving MCP server). Sealed store is read-only.
    let mut store = Store::open_with_root(&cache, &repo).unwrap();
    ga_query::indexer::build_index(&store, &repo).unwrap();
    store.commit_in_place().unwrap();
    store.seal_for_serving().unwrap();
    let ctx = McpContext::new(Arc::new(store));

    // External peer grabs the exclusive flock = "another MCP server is
    // reindexing this repo right now".
    let mut holder = spawn_exclusive_holder(&cache, &repo, 10);

    // Watcher fires a reindex. It loses the race → re-attaches read-only.
    // Pre-fix: build_index on the RO store errors → cell bricked.
    let result = ga_mcp::watcher::reindex_once(&ctx, &repo);

    // The store cell MUST remain populated — server still serves read-only,
    // not bricked. This is the assertion that fails pre-fix.
    assert!(
        ctx.try_store().is_ok(),
        "store cell must NOT be bricked after losing a reindex race; got {:?}",
        result
    );

    let _ = holder.kill();
    let _ = holder.wait();
}
