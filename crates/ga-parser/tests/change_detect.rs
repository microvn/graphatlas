//! S-005 AS-014 — detect per-file changes via BLAKE3 content hash.
//!
//! Classifies each file against a known-hash map into:
//!   - added: path not in known map
//!   - modified: path in known map but BLAKE3 differs
//!   - unchanged: path in known map and BLAKE3 matches
//!   - deleted: path in known map but file gone

use ga_parser::change_detect::{detect_changed_files, file_blake3, ChangeSet};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

#[test]
fn file_blake3_is_deterministic() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("x.py");
    write(&p, "print(1)");
    let a = file_blake3(&p).unwrap();
    let b = file_blake3(&p).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.len(), 32);
}

#[test]
fn blake3_differs_on_content_change() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("x.py");
    write(&p, "print(1)");
    let a = file_blake3(&p).unwrap();
    write(&p, "print(2)");
    let b = file_blake3(&p).unwrap();
    assert_ne!(a, b);
}

#[test]
fn first_run_classifies_all_as_added() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("a.py"), "x");
    write(&tmp.path().join("sub/b.py"), "y");
    let known: HashMap<PathBuf, [u8; 32]> = HashMap::new();
    let set: ChangeSet = detect_changed_files(tmp.path(), &known).unwrap();
    assert_eq!(set.added.len(), 2, "{:?}", set.added);
    assert!(set.modified.is_empty());
    assert!(set.unchanged.is_empty());
    assert!(set.deleted.is_empty());
}

#[test]
fn unchanged_files_classified_unchanged() {
    let tmp = TempDir::new().unwrap();
    let a_path = tmp.path().join("a.py");
    write(&a_path, "x");
    let mut known = HashMap::new();
    known.insert(PathBuf::from("a.py"), file_blake3(&a_path).unwrap());

    let set = detect_changed_files(tmp.path(), &known).unwrap();
    assert_eq!(set.unchanged.len(), 1);
    assert_eq!(set.unchanged[0].to_string_lossy(), "a.py");
    assert!(set.added.is_empty());
    assert!(set.modified.is_empty());
}

#[test]
fn modified_files_classified_modified() {
    let tmp = TempDir::new().unwrap();
    let a_path = tmp.path().join("a.py");
    write(&a_path, "old");
    let old_hash = file_blake3(&a_path).unwrap();
    write(&a_path, "new content here");

    let mut known = HashMap::new();
    known.insert(PathBuf::from("a.py"), old_hash);

    let set = detect_changed_files(tmp.path(), &known).unwrap();
    assert_eq!(set.modified.len(), 1);
    assert_eq!(set.modified[0].to_string_lossy(), "a.py");
    assert!(set.added.is_empty());
    assert!(set.unchanged.is_empty());
}

#[test]
fn deleted_files_classified_deleted() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("still_here.py"), "");

    let mut known = HashMap::new();
    known.insert(PathBuf::from("still_here.py"), [0u8; 32]);
    known.insert(PathBuf::from("gone.py"), [1u8; 32]);

    let set = detect_changed_files(tmp.path(), &known).unwrap();
    assert_eq!(set.deleted.len(), 1);
    assert_eq!(set.deleted[0].to_string_lossy(), "gone.py");
    // still_here.py hashes to something != [0u8; 32] → modified.
    assert_eq!(set.modified.len(), 1);
}

#[test]
fn empty_repo_empty_known_is_all_empty() {
    let tmp = TempDir::new().unwrap();
    let known: HashMap<PathBuf, [u8; 32]> = HashMap::new();
    let set = detect_changed_files(tmp.path(), &known).unwrap();
    assert!(set.added.is_empty());
    assert!(set.modified.is_empty());
    assert!(set.unchanged.is_empty());
    assert!(set.deleted.is_empty());
}

#[test]
fn excluded_dirs_skipped() {
    // Regression guard: change_detect must reuse walk_repo's exclude list.
    // node_modules content should never appear in the ChangeSet.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join("node_modules/react/index.js"), "");
    let known: HashMap<PathBuf, [u8; 32]> = HashMap::new();
    let set = detect_changed_files(tmp.path(), &known).unwrap();
    let names: Vec<String> = set
        .added
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with("app.py"));
}
