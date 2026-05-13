//! Tools S-003 cluster B — imported_names populated + re_export flag.

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
fn entry_carries_imported_names() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("utils/format.py"),
        "def fmt(): pass\ndef other(): pass\n",
    );
    write(&repo.join("a.py"), "from utils.format import fmt, other\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "utils/format.py").unwrap();
    assert_eq!(resp.importers.len(), 1);
    let mut names = resp.importers[0].imported_names.clone();
    names.sort();
    assert_eq!(names, vec!["fmt".to_string(), "other".to_string()]);
    assert!(!resp.importers[0].re_export);
}

#[test]
fn reexport_entry_flags_re_export_true() {
    // foo.ts re-exports from bar.ts with `export * from './bar'`.
    // ga_importers('bar.ts') must surface foo.ts as an importer with
    // re_export=true.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("bar.ts"), "export function b() {}\n");
    write(&repo.join("foo.ts"), "export * from './bar';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "bar.ts").unwrap();
    assert_eq!(resp.importers.len(), 1, "{:?}", resp.importers);
    assert_eq!(resp.importers[0].path, "foo.ts");
    assert!(
        resp.importers[0].re_export,
        "foo.ts→bar.ts must mark re_export: {:?}",
        resp.importers[0]
    );
}

#[test]
fn named_reexport_captures_imported_names() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("bar.ts"),
        "export function b() {}\nexport function c() {}\n",
    );
    write(&repo.join("foo.ts"), "export { b } from './bar';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "bar.ts").unwrap();
    assert_eq!(resp.importers.len(), 1);
    assert!(resp.importers[0].re_export);
    assert!(
        resp.importers[0].imported_names.contains(&"b".to_string()),
        "{:?}",
        resp.importers[0].imported_names
    );
}

#[test]
fn regular_import_does_not_set_re_export() {
    // Regression: normal TS `import { x } from './y'` must NOT be flagged.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("y.ts"), "export function y() {}\n");
    write(&repo.join("a.ts"), "import { y } from './y';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "y.ts").unwrap();
    assert_eq!(resp.importers.len(), 1);
    assert!(!resp.importers[0].re_export);
    assert!(resp.importers[0].imported_names.contains(&"y".to_string()));
}
