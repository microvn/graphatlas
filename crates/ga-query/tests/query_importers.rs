//! Tools S-003 cluster A — ga_importers AS-006 basic shape.

use ga_index::Store;
use ga_query::{importers, indexer::build_index};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn returns_all_direct_importers() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("utils/format.py"), "def fmt(): pass\n");
    write(&repo.join("a.py"), "from utils.format import fmt\n");
    write(&repo.join("b.py"), "from utils.format import fmt\n");
    write(&repo.join("c.py"), "from utils.format import fmt\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "utils/format.py").unwrap();
    assert_eq!(resp.importers.len(), 3);
    let mut paths: Vec<String> = resp.importers.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["a.py".to_string(), "b.py".to_string(), "c.py".to_string()]
    );
}

#[test]
fn entry_shape_matches_spec() {
    // path, import_line present; imported_names vec exists (may be empty in
    // cluster A — populated in cluster B).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("utils/format.py"), "def fmt(): pass\n");
    // Blank line 1, import on line 2.
    write(&repo.join("a.py"), "\nfrom utils.format import fmt\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "utils/format.py").unwrap();
    assert_eq!(resp.importers.len(), 1);
    let e = &resp.importers[0];
    assert_eq!(e.path, "a.py");
    assert_eq!(e.import_line, 2);
    // Vec exists; cluster A may leave it empty.
    assert!(e.imported_names.len() <= 32);
}

#[test]
fn unknown_file_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def f(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "nope.py").unwrap();
    assert!(resp.importers.is_empty());
}

#[test]
fn reject_non_safe_path_returns_empty() {
    // Cypher-injection defence (Tools-C9-d): reject path with quote/newline.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def f(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "a.py'\n DROP").unwrap();
    assert!(resp.importers.is_empty());
}
