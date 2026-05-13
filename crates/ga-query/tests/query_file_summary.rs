//! Tools S-005 cluster A — ga_file_summary AS-011.

use ga_index::Store;
use ga_query::{file_summary, indexer::build_index};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

#[test]
fn summary_returns_symbols_ordered_by_line() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        // Lines: 1 def a, 3 def b, 5 def c
        "def a(): pass\n\ndef b(): pass\n\ndef c(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = file_summary(&store, "m.py").unwrap();
    assert_eq!(resp.path, "m.py");
    let names: Vec<String> = resp.symbols.iter().map(|s| s.name.clone()).collect();
    assert_eq!(
        names,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    let lines: Vec<u32> = resp.symbols.iter().map(|s| s.line).collect();
    assert_eq!(lines, vec![1, 3, 5]);
}

#[test]
fn summary_includes_imports() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("utils/format.py"), "def fmt(): pass\n");
    write(&repo.join("a.py"), "from utils.format import fmt\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = file_summary(&store, "a.py").unwrap();
    assert_eq!(resp.imports, vec!["utils/format.py".to_string()]);
}

#[test]
fn summary_exports_contains_symbol_names() {
    // No per-lang visibility data yet — exports surface the defined symbol
    // names (sufficient for blast-radius prompts). Shape-compat with spec.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def a(): pass\ndef b(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = file_summary(&store, "m.py").unwrap();
    let mut exports = resp.exports.clone();
    exports.sort();
    assert_eq!(exports, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn summary_unknown_file_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def a(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = file_summary(&store, "missing.py").unwrap();
    assert_eq!(resp.path, "missing.py");
    assert!(resp.symbols.is_empty());
    assert!(resp.imports.is_empty());
    assert!(resp.exports.is_empty());
}

#[test]
fn summary_rejects_non_safe_path() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def a(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = file_summary(&store, "m.py'\n DROP").unwrap();
    assert!(resp.symbols.is_empty());
    assert!(resp.imports.is_empty());
    assert!(resp.exports.is_empty());
}
