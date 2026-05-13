//! Cross-process file lock — kernel-released flock (replaces v1 PID-file lock).
//!
//! The "Held" error path is preserved for backward-compat error text, but the
//! actual liveness gate is the kernel: flock(2) on Unix, LockFileEx on Windows.
//! Stale-PID can't happen by construction — fd-close releases the lock.

use ga_index::cache::CacheLayout;
use ga_index::lock::{LockError, LockFile, LockMode};
use std::path::Path;
use tempfile::TempDir;

fn layout(tmp: &TempDir, repo: &str) -> CacheLayout {
    let root = tmp.path().join(".graphatlas");
    let l = CacheLayout::for_repo(&root, Path::new(repo));
    l.ensure_dir().unwrap();
    l
}

#[test]
fn acquire_on_empty_cache_succeeds() {
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-a");
    let _lock = LockFile::acquire(&l, "gen-1").expect("fresh lock should succeed");
    assert!(l.lock_pid().exists());
}

#[test]
fn release_removes_file() {
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-b");
    let lock = LockFile::acquire(&l, "gen-1").unwrap();
    lock.release().unwrap();
    assert!(!l.lock_pid().exists());
}

#[test]
fn held_error_text_matches_spec_literal() {
    // AS-026: "Another graphatlas instance (PID N, started <duration> ago) is
    // indexing this repo. Wait for completion or kill PID N."
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-msg");
    let _first = LockFile::acquire(&l, "gen-1").unwrap();
    let err = match LockFile::acquire(&l, "gen-2") {
        Ok(_) => panic!("must refuse live lock"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(msg.contains("Another graphatlas instance"), "msg: {msg}");
    assert!(msg.contains("PID"), "msg: {msg}");
    assert!(
        msg.contains(" ago"),
        "expected humanized duration, got: {msg}"
    );
    assert!(msg.contains("Wait for completion"), "msg: {msg}");
    assert!(msg.contains("kill PID"), "msg: {msg}");
}

#[test]
fn second_exclusive_acquire_refuses() {
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-c");
    let _first = LockFile::acquire(&l, "gen-1").unwrap();

    let err = match LockFile::acquire(&l, "gen-2") {
        Ok(_) => panic!("must refuse second exclusive"),
        Err(e) => e,
    };
    match err {
        LockError::Held { pid, hostname, .. } => {
            assert_eq!(pid, std::process::id());
            assert_eq!(hostname, hostname::get().unwrap().to_string_lossy());
        }
        other => panic!("expected Held, got {other:?}"),
    }
}

#[test]
fn shared_lock_blocks_exclusive() {
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-shared-blocks-excl");
    let _reader = LockFile::try_acquire_shared(&l, "gen-r").unwrap();
    assert_eq!(_reader.mode(), LockMode::Shared);

    let err = LockFile::try_acquire_exclusive(&l, "gen-w").unwrap_err();
    assert!(matches!(err, LockError::Held { .. }));
}

#[test]
fn exclusive_lock_blocks_shared() {
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-excl-blocks-shared");
    let _writer = LockFile::try_acquire_exclusive(&l, "gen-w").unwrap();

    let err = LockFile::try_acquire_shared(&l, "gen-r").unwrap_err();
    assert!(matches!(err, LockError::Held { .. }));
}

#[test]
fn release_writer_then_attach_shared() {
    // Writer commits + drops → shared lock should now succeed.
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-release-attach");
    {
        let writer = LockFile::try_acquire_exclusive(&l, "gen-w").unwrap();
        drop(writer);
    }
    let reader = LockFile::try_acquire_shared(&l, "gen-r")
        .expect("shared after writer release should succeed");
    assert_eq!(reader.mode(), LockMode::Shared);
}

#[test]
fn corrupt_sidecar_does_not_block_exclusive() {
    // The sidecar is purely diagnostic — flock state is the kernel's. A
    // corrupt or stale JSON file from a crashed v1 lock must not poison
    // a fresh acquire.
    let tmp = TempDir::new().unwrap();
    let l = layout(&tmp, "/work/lock-corrupt-sidecar");
    ga_index::cache::write_file_0600(&l.lock_pid(), b"not json at all").unwrap();
    let _ = LockFile::acquire(&l, "ours").expect("corrupt sidecar must not block fresh acquire");
}
