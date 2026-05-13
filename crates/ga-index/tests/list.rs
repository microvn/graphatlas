//! S-003 AS-028 — `graphatlas list` library contract.
//!
//! Iterates cache-root, reads each cache's metadata.json, returns rows with
//! repo path (from metadata.repo_root), size-on-disk, and last-indexed wall clock.
//! Binary-level wiring is in tests/build_smoke.rs via the `list` subcommand.

use ga_index::list::{list_caches, CacheEntry};
use ga_index::Store;
use tempfile::TempDir;

#[test]
fn empty_cache_root_returns_empty_list() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&root).unwrap();
    let entries = list_caches(&root).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn missing_cache_root_returns_empty_list() {
    // Fresh machine — never ran graphatlas before.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas-never-existed");
    let entries = list_caches(&root).unwrap();
    assert!(entries.is_empty());
}

/// Create a real subdirectory under `tmp` to use as a repo fixture.
/// v1.5 PR2 foundation S-001 AS-001: `commit()` now strictly populates
/// `indexed_root_hash` via `compute_root_hash(repo_root)`, which requires
/// the path to exist on disk. Legacy `/work/<name>` string fixtures must
/// be materialized as real directories.
fn real_repo(tmp: &TempDir, rel: &str) -> std::path::PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    p
}

#[test]
fn lists_multiple_caches_across_different_repos() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas");
    // Materialize each repo under `tmp/repos/<rel>` so AS-001's Merkle
    // root-hash compute succeeds. metadata.repo_root captures the
    // canonical path which we recover for assertions below.
    let repo_paths: Vec<std::path::PathBuf> = ["client1/billing-api", "client2/billing-api", "notes"]
        .iter()
        .map(|r| real_repo(&tmp, r))
        .collect();
    for repo in &repo_paths {
        Store::open_with_root(&root, repo).unwrap().commit().unwrap();
    }
    let mut entries: Vec<CacheEntry> = list_caches(&root).unwrap();
    entries.sort_by(|a, b| a.repo_root.cmp(&b.repo_root));
    assert_eq!(entries.len(), 3);

    // Each entry's repo_root must equal one of the materialized fixtures.
    let mut expected: Vec<String> = repo_paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    expected.sort();
    let actual: Vec<String> = entries.iter().map(|e| e.repo_root.clone()).collect();
    assert_eq!(actual, expected);

    // Dir names follow `<repo-name>-<short-hash>` convention. Each repo's
    // basename — billing-api / billing-api / notes — must prefix one entry.
    for e in &entries {
        assert!(e.dir_name.starts_with("billing-api-") || e.dir_name.starts_with("notes-"));
        assert!(e.size_bytes > 0, "cache should have some size on disk");
        assert!(e.last_indexed_unix > 0);
    }
}

#[test]
fn ignores_non_graphatlas_subdirs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas");
    let real = real_repo(&tmp, "real");
    // First valid cache — also creates `root` with 0700 via ensure_cache_root.
    Store::open_with_root(&root, &real).unwrap().commit().unwrap();
    // Garbage directory with no metadata.json.
    std::fs::create_dir_all(root.join("garbage-dir")).unwrap();
    // Garbage directory with corrupt metadata.json.
    std::fs::create_dir_all(root.join("corrupt-dir")).unwrap();
    std::fs::write(root.join("corrupt-dir/metadata.json"), "{not json").unwrap();

    let entries = list_caches(&root).unwrap();
    // Only the real one. Corrupt + empty dirs are skipped silently (AS-028 only
    // requires enumeration of valid caches; `doctor` is where surface of bad
    // entries lives per Foundation AS-006).
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].repo_root, real.display().to_string());
}
