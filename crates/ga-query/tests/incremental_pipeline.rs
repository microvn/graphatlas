//! v1.5 PR9 triggers S-004 — incremental pipeline tests.
//!
//! Covers AS-012 dirty-path detection (sha256-vs-disk fallback per the
//! AS-012 gix-spike-deferral clause), AS-013 / AS-013b sha256 snapshot
//! invariant, AS-014 dependent expansion + truncation cap, and AS-015 /
//! AS-016 compile-time gate.

use ga_index::Store;
use ga_query::incremental::{
    dirty_paths, expand_dependents, plan, snapshot_all_indexed_sha256, IncrementalPlan,
    INCREMENTAL_ENABLED, MAX_REPARSE_FILES,
};
use ga_query::indexer::build_index;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn fresh_store_for(tmp: &TempDir, repo_subdir: &str) -> (Store, PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repos").join(repo_subdir);
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src").join("a.rs"), "fn a() {}\n").unwrap();
    fs::write(repo.join("src").join("b.rs"), "fn b() { super::a::a(); }\n").unwrap();
    let mut store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).expect("initial build");
    store.commit_in_place().unwrap();
    (store, repo)
}

// =====================================================================
// AS-012 — dirty path detection
// =====================================================================

#[test]
fn as_012_no_disk_changes_yields_empty_dirty_set() {
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "clean");
    let dirty = dirty_paths(&store, &repo).expect("dirty_paths must succeed");
    assert!(
        dirty.is_empty(),
        "AS-012: no disk changes must yield empty dirty set; got {dirty:?}"
    );
}

#[test]
fn as_012_file_content_modification_appears_in_dirty_set() {
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "modify");
    // Modify one file's CONTENT (mtime alone is not enough — see AS-013).
    fs::write(repo.join("src").join("a.rs"), "fn a() { /* edited */ }\n").unwrap();
    let dirty = dirty_paths(&store, &repo).expect("dirty_paths must succeed");
    assert!(
        dirty.iter().any(|p| p.to_string_lossy().ends_with("a.rs")),
        "AS-012: modified file must appear; got {dirty:?}"
    );
    assert!(
        !dirty.iter().any(|p| p.to_string_lossy().ends_with("b.rs")),
        "AS-012: unchanged file must NOT appear; got {dirty:?}"
    );
}

#[test]
fn as_012_newly_added_file_appears_in_dirty_set() {
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "add");
    fs::write(repo.join("src").join("c.rs"), "fn c() {}\n").unwrap();
    let dirty = dirty_paths(&store, &repo).expect("dirty_paths must succeed");
    assert!(
        dirty.iter().any(|p| p.to_string_lossy().ends_with("c.rs")),
        "AS-012: new file must appear; got {dirty:?}"
    );
}

#[test]
fn as_012_deleted_file_appears_in_dirty_set_for_pending_row_removal() {
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "delete");
    fs::remove_file(repo.join("src").join("a.rs")).unwrap();
    let dirty = dirty_paths(&store, &repo).expect("dirty_paths must succeed");
    assert!(
        dirty.iter().any(|p| p.to_string_lossy().ends_with("a.rs")),
        "AS-012: deleted file must surface for DELETE; got {dirty:?}"
    );
}

// =====================================================================
// AS-013 — sha256 skip-if-unchanged (mtime-only touch is a no-op)
// =====================================================================

#[test]
fn as_013_mtime_only_touch_does_not_make_file_dirty() {
    use std::time::SystemTime;
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "mtime-only");
    let file = repo.join("src").join("a.rs");
    // Read-modify-write with IDENTICAL content. mtime advances but
    // BLAKE3 hash is unchanged → must NOT be dirty.
    let content = fs::read(&file).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&file, &content).unwrap();
    // Force atime/mtime forward explicitly via filetime if needed —
    // sleep + rewrite is enough on macOS+linux test boxes.
    let _ = SystemTime::now();
    let dirty = dirty_paths(&store, &repo).expect("dirty_paths must succeed");
    assert!(
        dirty.is_empty(),
        "AS-013: identical-content rewrite must NOT mark file dirty; got {dirty:?}"
    );
}

#[test]
fn as_013_snapshot_read_takes_pre_modification_baseline() {
    let tmp = TempDir::new().unwrap();
    let (store, _repo) = fresh_store_for(&tmp, "snapshot");
    let map = snapshot_all_indexed_sha256(&store).expect("snapshot must succeed");
    assert!(
        map.len() >= 2,
        "AS-013: snapshot must populate from File rows; got {} entries",
        map.len()
    );
    // Every entry must be a non-zero hash (build_index populated them).
    for (path, hash) in &map {
        assert_ne!(
            hash,
            &[0u8; 32],
            "AS-013: indexed file {path:?} must have a non-zero sha256 in snapshot"
        );
    }
}

// =====================================================================
// AS-013b — post-cycle sha256 invariant (already covered by indexer's
//           File.sha256 population; this test asserts the snapshot
//           function returns the same hash a subsequent BLAKE3 of disk
//           would compute, so the closed-loop is verified end-to-end).
// =====================================================================

#[test]
fn as_013b_snapshot_hash_equals_disk_blake3_for_unchanged_file() {
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "invariant");
    let map = snapshot_all_indexed_sha256(&store).unwrap();
    let path_key = PathBuf::from("src/a.rs");
    let snapshot_hash = map
        .get(&path_key)
        .copied()
        .expect("AS-013b: src/a.rs must have an indexed hash");
    let disk_bytes = fs::read(repo.join("src").join("a.rs")).unwrap();
    let disk_hash: [u8; 32] = *blake3::hash(&disk_bytes).as_bytes();
    assert_eq!(
        snapshot_hash, disk_hash,
        "AS-013b: indexed sha256 must equal BLAKE3 of current disk content"
    );
}

// =====================================================================
// AS-014 — dependent expansion + cap
// =====================================================================

#[test]
fn as_014_empty_seed_returns_no_dependents() {
    let tmp = TempDir::new().unwrap();
    let (store, _repo) = fresh_store_for(&tmp, "empty-seed");
    let (deps, truncated) = expand_dependents(&store, &[]).unwrap();
    assert!(deps.is_empty());
    assert!(!truncated);
}

#[test]
fn as_014_truncation_flag_fires_when_seed_already_exceeds_cap() {
    let tmp = TempDir::new().unwrap();
    let (store, _repo) = fresh_store_for(&tmp, "cap");
    let seeds: Vec<PathBuf> = (0..600)
        .map(|i| PathBuf::from(format!("src/file_{i}.rs")))
        .collect();
    let (_deps, truncated) = expand_dependents(&store, &seeds).unwrap();
    assert!(
        truncated,
        "AS-014: seed count {} > cap {} must set truncated=true",
        seeds.len(),
        MAX_REPARSE_FILES
    );
}

// =====================================================================
// AS-015 + AS-016 — gate + fallback
// =====================================================================

#[test]
fn as_016_incremental_disabled_planner_returns_none_for_safe_fallback() {
    // INCREMENTAL_ENABLED is `false` by default (no Phase F artifact
    // wired yet). plan() MUST return None so the caller falls back to
    // ga_reindex full rebuild — that's the safe-by-default contract.
    assert_eq!(
        INCREMENTAL_ENABLED, false,
        "AS-016: default INCREMENTAL_ENABLED must be false until Phase F gate ships"
    );
    let tmp = TempDir::new().unwrap();
    let (store, repo) = fresh_store_for(&tmp, "fallback");
    let result = plan(&store, &repo).expect("plan must not error");
    assert!(
        result.is_none(),
        "AS-016: with INCREMENTAL_ENABLED=false, plan() must return None"
    );
}

// =====================================================================
// IncrementalPlan reparse_set ordering
// =====================================================================

#[test]
fn reparse_set_returns_changed_then_dependents() {
    let plan = IncrementalPlan {
        changed: vec![PathBuf::from("c.rs"), PathBuf::from("a.rs")],
        dependents: vec![PathBuf::from("b.rs")],
        truncated: false,
    };
    let set = plan.reparse_set();
    // Method preserves insertion order (changed first, then deps).
    assert_eq!(set[0], PathBuf::from("c.rs"));
    assert_eq!(set[1], PathBuf::from("a.rs"));
    assert_eq!(set[2], PathBuf::from("b.rs"));
}
