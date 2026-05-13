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
    // metadata stays in `Building` state. A second concurrent open cannot
    // safely attach read-only because the writer's lbug DB may be mid-write.
    // Caller should retry once the initial build completes.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/concurrent-building");
    let _first = Store::open_with_root(&cache_root, repo).unwrap();

    let err = Store::open_with_root(&cache_root, repo)
        .err()
        .expect("should refuse — metadata still Building");
    let s = format!("{err}");
    assert!(s.contains("indexing") || s.contains("retry"), "err: {s}");
}

#[test]
fn second_open_after_commit_attaches_read_only() {
    // Multi-terminal MCP scenario: first instance finishes initial build and
    // commits metadata. Second instance (e.g. another Claude Code terminal in
    // the same cwd) should attach as a read-only reader against the committed
    // cache instead of failing.
    use ga_index::store::OpenOutcome;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "concurrent-attach");
    let repo = repo_path.as_path();

    let mut first = Store::open_with_root(&cache_root, repo).unwrap();
    first.commit_in_place().unwrap();

    let second = Store::open_with_root(&cache_root, repo)
        .expect("second open after writer commit must attach read-only");
    assert!(second.is_read_only(), "second Store should be read-only");
    match second.outcome() {
        OpenOutcome::AttachedReadOnly { .. } => {}
        other => panic!("expected AttachedReadOnly, got {other:?}"),
    }
}

#[test]
fn read_only_store_refuses_commit() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_path = real_repo(&tmp, "ro-refuse-commit");
    let repo = repo_path.as_path();

    let mut first = Store::open_with_root(&cache_root, repo).unwrap();
    first.commit_in_place().unwrap();

    let mut second = Store::open_with_root(&cache_root, repo).unwrap();
    let err = second.commit_in_place().expect_err("must refuse commit");
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
