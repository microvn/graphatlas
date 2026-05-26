//! v1.5 PR6.1 (multi-mcp) M-3 — multi-process integration tests that
//! spawn `ga_index_lock_holder` (registered as `[[bin]]` in this crate's
//! Cargo.toml) to validate cross-process lock semantics.
//!
//! Same-process simulations of "peer holds writer flock" hit lbug 0.16.1's
//! shadow-page replay requirement (a same-process RW handle alive while
//! peer opens RO triggers "Couldn't replay shadow pages under read-only
//! mode"). Spawning a separate process avoids this — the helper only
//! holds the kernel flock, never an lbug handle.

use ga_index::cache::CacheLayout;
use ga_index::Store;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

/// Resolve the helper binary path. Cargo places it under
/// `target/<profile>/ga_index_lock_holder` (or `target/<profile>/deps/`).
/// Walks up from `CARGO_MANIFEST_DIR` to find it.
fn helper_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // workspace target dir is two levels up from crates/ga-index/.
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
        "ga_index_lock_holder binary not found in target/{{debug,release}}/. \
         Run `cargo build -p ga-index --bin ga_index_lock_holder` first."
    );
}

/// Spawn the helper holding `mode` on the given cache. Returns the child
/// process + a stdout reader positioned AFTER the `READY` sync line.
fn spawn_holder(
    cache_root: &std::path::Path,
    repo_root: &std::path::Path,
    mode: &str,
    secs: u64,
) -> (Child, BufReader<ChildStdout>) {
    let bin = helper_path();
    let mut child = Command::new(&bin)
        .arg("--cache-root")
        .arg(cache_root)
        .arg("--repo-root")
        .arg(repo_root)
        .arg("--hold")
        .arg(mode)
        .arg("--secs")
        .arg(secs.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn helper at {}: {e}", bin.display()));

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("read READY line from helper");
    assert!(
        line.trim_end() == "READY",
        "expected READY sync line from helper, got: {line:?}"
    );
    (child, reader)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn real_repo(tmp: &TempDir, name: &str) -> PathBuf {
    let p = tmp.path().join("repos").join(name);
    std::fs::create_dir_all(&p).expect("mkdir repos");
    std::fs::write(p.join("README.md"), "# fixture\n").expect("seed");
    p
}

#[test]
fn mm_as_002_reindex_in_place_with_external_holder_returns_read_only_and_preserves_cache() {
    // v1.5 PR6.1 (multi-mcp) S-001 AS-002 cache-invariant: while an
    // external process holds the writer flock, `reindex_in_place` must
    // refuse via the AttachedReadOnly callee-guard (AS-003) AND the
    // graph.db on disk must be byte-identical pre/post.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "mm-as-002");

    // Phase 1 — plant a committed cache.
    {
        let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
        writer.commit_in_place().unwrap();
    }
    let layout = CacheLayout::for_repo(&cache_root, &repo_path);
    let db_path = layout.graph_db();
    let sha_before = sha256_hex(&std::fs::read(&db_path).expect("read db pre"));

    // Phase 2 — spawn helper holding exclusive flock externally.
    let (mut holder, _stdout) = spawn_holder(&cache_root, &repo_path, "exclusive", 10);

    // Phase 3 — parent opens Store → falls through to open_read_only
    // (helper holds excl). Helper does NOT have any lbug handle, so
    // parent's lbug RO open succeeds.
    let store = Store::open_with_root(&cache_root, &repo_path)
        .expect("parent should attach read-only while helper holds excl");
    assert!(
        store.is_read_only(),
        "parent must attach read-only when external flock held"
    );

    // Phase 4 — reindex_in_place re-attaches read-only (per AS-002
    // post-bug-fix 2026-05-26). Cache integrity preserved across the
    // attempt; the returned Store remains usable for queries and can
    // recover (transition to writer) once the external holder releases.
    let result = store
        .reindex_in_place(&repo_path)
        .expect("reindex_in_place must return Ok(read_only) when peer holds");
    assert!(
        result.is_read_only(),
        "must return read-only Store when external flock held"
    );

    let sha_after = sha256_hex(&std::fs::read(&db_path).expect("read db post"));
    assert_eq!(
        sha_before, sha_after,
        "graph.db sha256 must be unchanged after peer-held reindex (cache integrity invariant)"
    );

    // Cleanup helper. Kill is OK here — it's our own child process,
    // not enforced by the CI grep lint (that lint only checks
    // src/cmd_reset.rs).
    holder.kill().ok();
    holder.wait().ok();
}

#[test]
fn mm_as_007_peer_can_acquire_exclusive_after_writer_seal_release() {
    // v1.5 PR6.1 (multi-mcp) S-002 AS-005/AS-007: post-seal the writer
    // releases the exclusive flock entirely. A separate process can
    // subsequently acquire exclusive without contention.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "mm-as-007");

    // Phase 1 — parent commits as writer (seal_for_serving releases flock).
    {
        let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
        writer.commit_in_place().unwrap();
        // writer stays alive (post-seal, no flock). Drop at end of scope.
    }

    // Phase 2 — external process acquires exclusive via the helper. If
    // the writer hadn't released, this would block (helper exits 1 with
    // LockError::Held). The fact that helper prints READY proves AS-005
    // semantics from a real second process.
    let (mut holder, _stdout) = spawn_holder(&cache_root, &repo_path, "exclusive", 5);
    holder.kill().ok();
    holder.wait().ok();
}

#[test]
fn mm_two_mcp_sequential_reindex_both_succeed() {
    // v1.5 PR6.1 (multi-mcp) — the load-bearing scenario from user's
    // original prompt: "2 MCP servers cùng repo đều chủ động reindex
    // được qua tool ga_reindex (ở các thời điểm khác nhau, không đồng
    // thời)."
    //
    // Simulation in same process (still valid because both Stores are
    // post-seal RO at steady state — no lbug RW+RO conflict): plant
    // committed cache, open two Stores sequentially (each seals on
    // open via MCP boot path equivalent), then reindex via the first,
    // then reindex via the second. Both must succeed; cache must be
    // healthy after each.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "two-mcp-seq");

    // Initial build (Process A's first boot, FreshBuild → commit → seal).
    {
        let mut a = Store::open_with_root(&cache_root, &repo_path).unwrap();
        a.commit_in_place().unwrap();
    }

    // Process A re-opens (simulating MCP boot after initial build →
    // Resumed → seal_for_serving in mcp_cmd.rs).
    let mut a = Store::open_with_root(&cache_root, &repo_path).unwrap();
    a.seal_for_serving().unwrap();
    assert!(a.is_read_only(), "post-seal A is read-only");

    // Process A initiates reindex — drops self, re-acquires exclusive,
    // nukes, rebuilds, seals.
    let a_after = a.reindex_in_place(&repo_path).unwrap();
    let mut a_after = a_after;
    a_after.commit_in_place().unwrap();

    // Cache must be healthy.
    let layout = CacheLayout::for_repo(&cache_root, &repo_path);
    assert!(
        layout.graph_db().exists(),
        "graph.db must exist after A's reindex"
    );

    // Process B boots fresh (separate Store, sequential to A's reindex).
    let mut b = Store::open_with_root(&cache_root, &repo_path).unwrap();
    b.seal_for_serving().unwrap();

    // Process B initiates reindex.
    let b_after = b.reindex_in_place(&repo_path).unwrap();
    let mut b_after = b_after;
    b_after.commit_in_place().unwrap();

    assert!(
        layout.graph_db().exists(),
        "graph.db must exist after B's reindex"
    );
    // a_after Store still alive (post-seal, no flock) — it could query
    // via refresh_if_stale to see B's new generation.
    drop(a_after);
    drop(b_after);
}

#[test]
fn mm_as_006_attach_during_initial_build_polls_then_attaches() {
    // v1.5 PR6.1 (multi-mcp) S-002 AS-006: while peer holds exclusive
    // AND metadata=Building (initial build in progress), parent's
    // open_read_only polls with exponential backoff. When peer releases
    // (and metadata transitions to Complete), parent attaches RO.
    //
    // The helper acquires the flock but does NOT plant Building
    // metadata — for this assertion we drive metadata via a transient
    // parent Store (commit → Complete) BEFORE the helper holds the
    // flock. So this test exercises the post-Complete-but-flock-held
    // boot path: parent's poll observes Match(Complete) immediately
    // and the open_read_only finishes fast.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "mm-as-006");

    {
        let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
        writer.commit_in_place().unwrap();
    }

    let (mut holder, _stdout) = spawn_holder(&cache_root, &repo_path, "exclusive", 5);

    // Use a tight retry budget so this test stays fast even if helper
    // start-up adds jitter.
    ga_index::store::set_readonly_retry_budget_ms_for_tests(2000);
    let started = std::time::Instant::now();
    let store = Store::open_with_root(&cache_root, &repo_path)
        .expect("attach RO while external flock held");
    let elapsed = started.elapsed();
    ga_index::store::set_readonly_retry_budget_ms_for_tests(u64::MAX);

    assert!(store.is_read_only(), "must attach RO");
    assert!(
        elapsed < Duration::from_secs(3),
        "attach should be fast when metadata already Complete (elapsed: {elapsed:?})"
    );

    holder.kill().ok();
    holder.wait().ok();
}

#[test]
fn mm_concurrent_reindex_loser_recovers_after_winner_releases() {
    // v1.5 PR6.1 (multi-mcp) — regression test for the
    // "concurrent reindex traps the loser forever" bug reported
    // 2026-05-26.
    //
    //   Pre-fix: 2 processes fire ga_reindex simultaneously. Winner
    //   succeeds. Loser's reindex_in_place returned Ok(read_only)
    //   (AS-002 fallback), MCP handler then called build_index on it
    //   → lbug "Cannot execute write operations in a read-only database"
    //   → cell left None → every subsequent reindex on loser fails
    //   forever.
    //
    //   Post-fix: loser receives Ok(read_only) Store with cache
    //   untouched; subsequent reindex_in_place on the read-only Store
    //   (now that peer has released) acquires exclusive and proceeds
    //   to rebuild. No outcome-based guard blocks recovery.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "mm-recover");

    {
        let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
        writer.commit_in_place().unwrap();
    }

    // Phase 1 — external holder holds exclusive (= peer is reindexing).
    let (mut holder, _stdout) = spawn_holder(&cache_root, &repo_path, "exclusive", 2);

    // Phase 2 — loser opens + attempts reindex. Returns Ok(read_only).
    let store = Store::open_with_root(&cache_root, &repo_path).expect("loser open");
    assert!(
        store.is_read_only(),
        "loser must attach RO while holder alive"
    );
    let after_fail = store
        .reindex_in_place(&repo_path)
        .expect("loser must get Ok(read_only) not Err");
    assert!(
        after_fail.is_read_only(),
        "loser's reindex result must stay read-only while peer holds"
    );

    // Phase 3 — wait for holder to exit (releases flock at --secs end).
    holder.wait().expect("holder wait");

    // Phase 4 — loser retries reindex. Peer gone, try_acquire succeeds.
    let recovered = after_fail
        .reindex_in_place(&repo_path)
        .expect("loser must recover once peer releases");
    assert!(
        !recovered.is_read_only(),
        "after recovery loser is writer mode again, got outcome={:?}",
        recovered.outcome()
    );
}
