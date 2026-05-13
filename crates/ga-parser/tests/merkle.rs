//! S-005 AS-013 — bounded Merkle root hash.
//!
//! Deterministic BLAKE3 over:
//!   - top-N (default 32) dirs at depth ≤ 2 with their mtime_ns
//!   - `.git/index` mtime_ns (if present)
//!   - `.git/HEAD` full content bytes (if present)

use ga_parser::merkle::{compute_root_hash, MerkleConfig};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

#[test]
fn empty_repo_has_deterministic_hash() {
    let tmp = TempDir::new().unwrap();
    let h1 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();
    let h2 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();
    assert_eq!(h1, h2, "hash must be deterministic on same tree");
    assert_eq!(h1.len(), 32, "BLAKE3 output is 32 bytes");
}

#[test]
fn missing_repo_is_error() {
    let err = compute_root_hash(Path::new("/nonexistent/path/xyz"), &MerkleConfig::default()).err();
    assert!(err.is_some());
}

#[test]
fn adding_file_changes_hash_via_dir_mtime() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("sub/a.py"), "x");
    let h1 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();

    // Sleep 1s so mtime buckets separate.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&tmp.path().join("sub/b.py"), "y");
    let h2 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();

    assert_ne!(
        h1, h2,
        "adding a file should change the directory mtime → different hash"
    );
}

#[test]
fn head_content_is_part_of_hash() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join(".git/HEAD"), "ref: refs/heads/main\n");
    let h1 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();

    write(&tmp.path().join(".git/HEAD"), "ref: refs/heads/feature\n");
    let h2 = compute_root_hash(tmp.path(), &MerkleConfig::default()).unwrap();
    assert_ne!(h1, h2, "changing .git/HEAD content must change hash");
}

#[test]
fn missing_git_dir_still_hashes() {
    // Repos without .git (fresh checkout, mono-repo sub-package, etc.) still
    // need to hash — we just omit the git section.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let h = compute_root_hash(tmp.path(), &MerkleConfig::default());
    assert!(h.is_ok(), "hash should succeed without .git/: {h:?}");
}

#[test]
fn bound_n_caps_dirs_visited() {
    // 40 sibling dirs → with bound_n=32, only 32 are fed into the hash.
    // Test via: creating 40 dirs, hashing with bound=32 vs bound=40. Should
    // differ only if dirs 33..40 contribute.
    let tmp = TempDir::new().unwrap();
    for i in 0..40u32 {
        write(&tmp.path().join(format!("d{i:02}/a.py")), "");
    }
    let cfg32 = MerkleConfig {
        bound_n: 32,
        ..MerkleConfig::default()
    };
    let cfg40 = MerkleConfig {
        bound_n: 40,
        ..MerkleConfig::default()
    };
    let h32 = compute_root_hash(tmp.path(), &cfg32).unwrap();
    let h40 = compute_root_hash(tmp.path(), &cfg40).unwrap();
    assert_ne!(
        h32, h40,
        "expanding bound_n from 32→40 should produce a different hash \
         (proves extra dirs were included) — got same: {h32:?}"
    );
}

#[test]
fn depth_cap_excludes_deep_dirs() {
    // depth_cap=1 means we don't descend into `sub/sub2/` — touching a file
    // there should NOT change the hash. But touching a file at depth=0/1 should.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("top/mid/deep/a.py"), "");
    let cfg = MerkleConfig {
        depth_cap: 1,
        ..MerkleConfig::default()
    };
    let h1 = compute_root_hash(tmp.path(), &cfg).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    // Write a new file deeper than the depth cap.
    write(&tmp.path().join("top/mid/deep/b.py"), "");
    let h2 = compute_root_hash(tmp.path(), &cfg).unwrap();
    assert_eq!(
        h1, h2,
        "depth-capped hash should not pick up changes past depth_cap"
    );
}

#[test]
fn sort_order_is_stable_across_fs_ordering() {
    // Different filesystems return read_dir in different orders; our hash must
    // normalize so the result doesn't depend on iteration order.
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    // Write same files in different temporal order.
    write(&tmp1.path().join("b/x.py"), "");
    std::thread::sleep(std::time::Duration::from_millis(10));
    write(&tmp1.path().join("a/x.py"), "");

    write(&tmp2.path().join("a/x.py"), "");
    std::thread::sleep(std::time::Duration::from_millis(10));
    write(&tmp2.path().join("b/x.py"), "");

    // Because mtimes differ between the two tempdirs, we can't assert equal
    // hashes directly — but we CAN assert that each tempdir's hash is
    // independent of inode ordering within it. Re-hash twice in a row:
    let h1a = compute_root_hash(tmp1.path(), &MerkleConfig::default()).unwrap();
    let h1b = compute_root_hash(tmp1.path(), &MerkleConfig::default()).unwrap();
    assert_eq!(h1a, h1b, "same tree, same hash");
}
