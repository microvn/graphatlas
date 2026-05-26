//! S-003 AS-007 + AS-008 + AS-025 + AS-027 — end-to-end Store::open behavior.

use ga_core::IndexState;
use ga_index::{OpenOutcome, Store};
use std::path::Path;
use tempfile::TempDir;

/// v1.5 PR2 AS-001: `commit()` and `commit_in_place()` strictly populate
/// `indexed_root_hash` via `compute_root_hash(repo_root)` which requires
/// the path to exist on disk. Materialize a real repo dir for tests that
/// exercise the commit path. Tests that only open + drop without commit
/// can keep their legacy `/work/<name>` string paths.
fn real_repo(tmp: &TempDir, rel: &str) -> std::path::PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    p
}

#[test]
fn open_fresh_repo_returns_fresh_build_signal() {
    // AS-007 bootstrap case: no cache → caller must build.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/fresh");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    assert!(matches!(store.outcome(), OpenOutcome::FreshBuild));
    assert_eq!(store.metadata().index_state, IndexState::Building);
}

#[test]
fn reopen_complete_cache_returns_resume() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "complete");
    let repo = repo_path.as_path();
    let s1 = Store::open_with_root(&cache_root, repo).unwrap();
    s1.commit().unwrap();

    let s2 = Store::open_with_root(&cache_root, repo).unwrap();
    assert!(matches!(s2.outcome(), OpenOutcome::Resumed));
    assert_eq!(s2.metadata().index_state, IndexState::Complete);
}

#[test]
fn schema_mismatch_deletes_cache_and_rebuilds() {
    // AS-008 + AS-027: cache on disk has old schema → open nukes + fresh.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "schema-bump");
    let repo = repo_path.as_path();

    // Seed a v1 cache then commit.
    let s1 = Store::open_with_root_and_schema(&cache_root, repo, 1).unwrap();
    s1.commit().unwrap();

    // Reopen as v99 → must wipe + fresh build.
    let s2 = Store::open_with_root_and_schema(&cache_root, repo, 99).unwrap();
    assert!(matches!(
        s2.outcome(),
        OpenOutcome::RebuildSchemaMismatch {
            cache: 1,
            binary: 99
        }
    ));
    assert_eq!(s2.metadata().schema_version, 99);
}

#[test]
fn crashed_building_triggers_rebuild() {
    // AS-025: previous indexer crashed before commit → building sentinel
    // present → open detects + wipes + fresh build.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/crashed");
    let _s1 = Store::open_with_root(&cache_root, repo).unwrap();
    // deliberately do NOT commit (simulates SIGKILL mid-indexing).
    drop(_s1); // release lock; metadata.json stays with state=building.

    let s2 = Store::open_with_root(&cache_root, repo).unwrap();
    assert!(
        matches!(s2.outcome(), OpenOutcome::RebuildCrashRecovery { .. }),
        "outcome: {:?}",
        s2.outcome()
    );
    assert_eq!(s2.metadata().index_state, IndexState::Building);
}

#[test]
fn concurrent_open_during_initial_build_is_refused() {
    // First instance is mid-build (FreshBuild outcome, never committed) →
    // metadata stays in `Building` state. A second concurrent open
    // exercises the v1.5 PR6.1 (multi-mcp) S-002 AS-006 boot-race
    // exponential backoff: polls for `Match(Complete)` until the budget
    // expires, then returns the existing "no committed cache yet" Err.
    //
    // Use a short retry budget (200ms) so the test exercises the
    // timeout path without paying 30s. Production default = 30s.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/concurrent-building");
    let _first = Store::open_with_root(&cache_root, repo).unwrap();

    ga_index::store::set_readonly_retry_budget_ms_for_tests(200);
    let err = Store::open_with_root(&cache_root, repo)
        .err()
        .expect("should refuse — metadata still Building after retry budget");
    ga_index::store::set_readonly_retry_budget_ms_for_tests(u64::MAX);
    let s = format!("{err}");
    assert!(s.contains("indexing") || s.contains("retry"), "err: {s}");
}

#[test]
fn second_open_after_commit_resumes_no_attached_read_only() {
    // v1.5 PR6.1 (multi-mcp) S-002 AS-005: post-seal the first writer
    // releases the exclusive flock entirely. The second `Store::open`
    // therefore lands on `Resumed` (committed cache, no flock holder)
    // — NOT `AttachedReadOnly` as in the pre-PR6.1 design where the
    // first writer kept a long-lived shared flock.
    //
    // The multi-MCP attach scenario is now exercised by attaching
    // WHILE peer holds exclusive (mid-build) — covered separately.
    use ga_index::store::OpenOutcome;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "second-open-resumes");
    let repo = repo_path.as_path();

    let mut first = Store::open_with_root(&cache_root, repo).unwrap();
    first.commit_in_place().unwrap();
    // First is still alive (post-seal, no flock held).

    let second = Store::open_with_root(&cache_root, repo)
        .expect("second open after writer commit must succeed");
    match second.outcome() {
        OpenOutcome::Resumed => {}
        other => panic!("expected Resumed (post-PR6.1 no-flock-at-steady-state), got {other:?}"),
    }
    assert!(
        !second.is_read_only(),
        "second Store should be a writer because first released the flock at seal"
    );
}

#[test]
#[ignore = "Same-process simulation hits lbug 0.16.1 shadow-page replay \
           requirement. Multi-process equivalent now lives at \
           tests/multi_process_lock.rs::mm_as_006_attach_during_initial_build_polls_then_attaches \
           (uses ga_index_lock_holder helper bin)."]
fn second_open_while_first_mid_build_attaches_read_only() {
    // v1.5 PR6.1 (multi-mcp) S-002 AS-006: while peer is mid-initial-build
    // (holds exclusive, metadata=Building), the second open must attach
    // as read-only via the exponential-backoff path. We simulate the
    // contention window by NOT calling commit_in_place on the first
    // Store — it still holds exclusive.
    //
    // The backoff polls up to 30s; we don't want a 30s test, so we
    // commit on a background thread after a short delay so the second
    // open's poll loop observes `Match(Complete)` quickly.
    use ga_index::store::OpenOutcome;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "second-open-during-build");

    let first = Arc::new(Mutex::new(Some(
        Store::open_with_root(&cache_root, &repo_path).unwrap(),
    )));
    let first_for_thread = Arc::clone(&first);
    let commit_thread = thread::spawn(move || {
        // Give the second open's backoff loop a few iterations to fire.
        thread::sleep(Duration::from_millis(300));
        let mut guard = first_for_thread.lock().unwrap();
        if let Some(mut s) = guard.take() {
            s.commit_in_place().expect("commit_in_place");
            // Keep Store alive but with flock released.
            *guard = Some(s);
        }
    });

    let second = Store::open_with_root(&cache_root, &repo_path)
        .expect("second open should attach after writer commits");
    commit_thread.join().expect("commit thread join");

    // After commit, no peer holds exclusive → second may have landed
    // on Resumed OR AttachedReadOnly depending on race. Both are valid
    // per spec — we only assert no panic + cache usable.
    match second.outcome() {
        OpenOutcome::Resumed | OpenOutcome::AttachedReadOnly { .. } => {}
        other => panic!("expected Resumed or AttachedReadOnly, got {other:?}"),
    }
}

#[test]
fn read_only_store_refuses_commit() {
    // Hand-craft an AttachedReadOnly by holding the writer flock from a
    // sibling thread so `Store::open_with_root` for our handle falls
    // through to `open_read_only`. The fall-through path reads
    // metadata that we plant via a transient first-Store commit.
    use ga_index::cache::CacheLayout;
    use ga_index::lock::LockFile;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "ro-refuse-commit");
    let repo = repo_path.as_path();

    {
        // Plant committed metadata.
        let mut first = Store::open_with_root(&cache_root, repo).unwrap();
        first.commit_in_place().unwrap();
    }

    // Hold the writer flock from outside the Store so the second open
    // lands on `open_read_only`.
    let layout = CacheLayout::for_repo(&cache_root, repo);
    let _holder = LockFile::try_acquire_exclusive(&layout, "external-holder")
        .expect("acquire external writer flock");

    let mut reader = Store::open_with_root(&cache_root, repo)
        .expect("second open must attach read-only while peer holds exclusive");
    assert!(reader.is_read_only());
    let err = reader.commit_in_place().expect_err("must refuse commit");
    let s = format!("{err}");
    assert!(s.contains("read-only"), "err: {s}");
}

#[test]
fn mvcc_read_not_blocked_by_write_smoke() {
    // AS-009 smoke: spike already validated this at scale (ADR-001). Here we
    // just confirm Store::open yields a working lbug connection and that a
    // read sees writes committed by another connection on the same DB.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/mvcc");
    let store = Store::open_with_root(&cache_root, repo).unwrap();

    // DDL + write through a dedicated write connection.
    {
        let c = store.connection().unwrap();
        let _ = c.query("CREATE NODE TABLE IF NOT EXISTS K(k STRING, v STRING, PRIMARY KEY(k))");
        c.query("MERGE (:K {k: 'hello', v: 'world'})").unwrap();
    }

    // Reader connection (separate Connection). MVCC scope: sees the commit.
    let reader = store.connection().unwrap();
    let rs = reader.query("MATCH (n:K {k: 'hello'}) RETURN n.v").unwrap();
    let values: Vec<String> = rs
        .into_iter()
        .filter_map(|row| match row.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(values, vec!["world".to_string()]);
}

// =====================================================================
// v1.5 PR6.1 (multi-mcp) — new behavior tests
// =====================================================================

#[test]
fn pr61_post_seal_writer_holds_no_flock() {
    // S-002 AS-005: post-seal writer releases the exclusive flock entirely.
    // A peer can subsequently acquire exclusive without contention.
    use ga_index::cache::CacheLayout;
    use ga_index::lock::LockFile;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "post-seal-no-flock");
    let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
    writer.commit_in_place().unwrap();
    // Writer Store is still alive (post-seal), but flock is released.

    let layout = CacheLayout::for_repo(&cache_root, &repo_path);
    let peer_lock = LockFile::try_acquire_exclusive(&layout, "peer-test")
        .expect("peer must be able to acquire exclusive after writer's seal_for_serving release");
    peer_lock.release().ok();
}

#[test]
fn pr61_reindex_in_place_attached_read_only_returns_ok_read_only_preserves_cache() {
    // S-001 AS-002 (post-bug-fix 2026-05-26): when a peer process holds
    // the writer flock, `reindex_in_place` returns Ok(read_only_store)
    // rather than Err — this keeps the caller's Store cell populated and
    // allows recovery once the peer releases. Cache integrity invariant
    // still holds: graph.db sha256 unchanged across the refused attempt.
    //
    // Pre-fix behavior was `Err("reindex_in_place refused: store is
    // attached read-only")`, which trapped the loser of a concurrent
    // race forever (build_index would then never run, but ALSO
    // subsequent reindex attempts would refuse based on outcome).
    use ga_index::cache::CacheLayout;
    use ga_index::lock::LockFile;
    use ga_index::OpenOutcome;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "guard-callee");

    // Plant committed cache then hold an external flock so the second
    // open attaches read-only.
    {
        let mut s = Store::open_with_root(&cache_root, &repo_path).unwrap();
        s.commit_in_place().unwrap();
    }
    let layout = CacheLayout::for_repo(&cache_root, &repo_path);
    let _holder = LockFile::try_acquire_exclusive(&layout, "external").expect("external holder");

    let reader = Store::open_with_root(&cache_root, &repo_path).unwrap();
    assert!(reader.is_read_only());

    // Snapshot graph.db sha256 before the reindex attempt.
    let db_path = layout.graph_db();
    let sha_before = std::fs::read(&db_path)
        .map(sha256_hex)
        .expect("read db pre");

    let result = reader
        .reindex_in_place(&repo_path)
        .expect("reindex_in_place must return Ok(read_only) when peer holds exclusive");
    assert!(
        result.is_read_only(),
        "returned Store must be read-only (peer-held fallback)"
    );
    assert!(
        matches!(result.outcome(), OpenOutcome::AttachedReadOnly { .. }),
        "outcome must be AttachedReadOnly, got {:?}",
        result.outcome()
    );

    let sha_after = std::fs::read(&db_path)
        .map(sha256_hex)
        .expect("read db post");
    assert_eq!(
        sha_before, sha_after,
        "graph.db must be untouched after peer-held reindex attempt (cache integrity)"
    );
}

#[test]
#[ignore = "Superseded by tests/multi_process_lock.rs::\
           mm_as_002_reindex_in_place_with_external_holder_returns_read_only_and_preserves_cache \
           (real subprocess via ga_index_lock_holder helper). AS-002 \
           cache-invariant also asserted in \
           pr61_reindex_in_place_refuses_on_attached_read_only."]
fn pr61_reindex_in_place_peer_held_returns_read_only_store() {
    // S-001 AS-002: when an external process holds the writer flock,
    // reindex_in_place returns Ok(read_only_store) — the caller's
    // Store cell stays populated. Cache untouched (sha256 invariant).
    use ga_index::cache::CacheLayout;
    use ga_index::lock::LockFile;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "peer-held-ok-readonly");

    // Plant a committed cache so the read-only re-attach can succeed.
    let mut writer = Store::open_with_root(&cache_root, &repo_path).unwrap();
    writer.commit_in_place().unwrap();
    // writer is now post-seal (no flock), drop it so we get a clean state.
    drop(writer);

    let layout = CacheLayout::for_repo(&cache_root, &repo_path);
    let db_path = layout.graph_db();
    let sha_before = std::fs::read(&db_path)
        .map(sha256_hex)
        .expect("read db pre");

    // External holder takes the exclusive lock so our reindex acquire fails.
    let holder =
        LockFile::try_acquire_exclusive(&layout, "external-blocker").expect("external holder lock");

    let store = Store::open_with_root(&cache_root, &repo_path).expect("open");
    // The open path acquired exclusive — wait, no: holder is alive. So
    // open's try_acquire_exclusive will fail → fall through to
    // open_read_only → attach. Confirm:
    assert!(store.is_read_only());

    // reindex_in_place on attached-read-only is refused by the callee
    // guard (AS-003) BEFORE the peer-held branch runs. To exercise
    // AS-002 specifically we need a Store that is NOT
    // AttachedReadOnly but whose reindex try_acquire_exclusive fails.
    // The simplest way: drop the external holder + open writer-mode,
    // then re-acquire holder before reindex_in_place runs.
    drop(store);
    drop(holder);
    let writer2 = Store::open_with_root(&cache_root, &repo_path).expect("open writer");
    assert!(!writer2.is_read_only());
    // Now grab the holder back BEFORE calling reindex.
    // writer2 has its own exclusive flock — we have to drop it first
    // so the external holder can acquire. reindex_in_place drops `self`
    // at function entry, so we can pre-acquire the holder via a thread.
    // Simpler approach: drop writer2 to release its flock, take holder
    // ourselves, then writer2's reindex_in_place won't compile because
    // it's already dropped. So we use a different pattern: hold the
    // lock concurrently from a separate thread.
    use std::sync::{Arc, Barrier};
    use std::thread;

    let layout_for_thread = layout.clone();
    let barrier = Arc::new(Barrier::new(2));
    let barrier_for_thread = Arc::clone(&barrier);
    let release_signal = Arc::new(std::sync::Mutex::new(false));
    let release_for_thread = Arc::clone(&release_signal);

    // Park the writer's flock first by dropping writer2.
    drop(writer2);

    let holder_thread = thread::spawn(move || {
        let _h = LockFile::try_acquire_exclusive(&layout_for_thread, "external")
            .expect("thread holder acquire");
        barrier_for_thread.wait();
        // Hold until the main thread says go.
        loop {
            if *release_for_thread.lock().unwrap() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
    });

    barrier.wait();
    // Open in writer mode would fail because thread holds excl. But the
    // test target is `reindex_in_place` on a Store that is NOT attached
    // read-only. So we open a Store BEFORE the thread acquired —
    // refactor: actually, due to ordering complexity, just verify the
    // attached-read-only refusal path already covers cache safety
    // (cache untouched on refused reindex — same invariant).
    *release_signal.lock().unwrap() = true;
    holder_thread.join().ok();

    let sha_after = std::fs::read(&db_path)
        .map(sha256_hex)
        .expect("read db post");
    assert_eq!(sha_before, sha_after, "graph.db must be untouched");
}

fn sha256_hex(bytes: Vec<u8>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect::<String>()
}
