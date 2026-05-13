//! Gap 5 — AS-012 `Symbol.is_test_marker` per-language detection.
//!
//! Spec: spec, AS-012.
//! Per-lang AST attribute detection for parser-time test markers.
//!
//! Patterns per spec line 326 (Tools-C3):
//! - Rust: `#[test]` / `#[tokio::test]` / `#[cfg(test)]`-mod members
//! - Java/Kotlin: `@Test` annotation
//! - Python: `def test_*` naming convention
//! - JS/TS: function inside `describe(...)` / `it(...)` / `test(...)` block
//!   (deferred — runtime-style; test_path heuristic suffices for v1.3)
//! - Ruby: `def test_*` naming
//! - Go: `func TestX` naming
//! - C#: `[Test]` / `[Fact]` / `[TestMethod]` attribute (deferred — needs
//!   per-attribute name match)
//!
//! Tools-C3: parser-time `is_test_marker` complements runtime
//! `is_test_path` filename heuristic. UC consumers union/intersect per
//! AS-012 conventions.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn is_test_marker_of(store: &Store, name: &str) -> Option<bool> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.is_test_marker");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            return Some(b);
        }
    }
    None
}

#[test]
fn rust_test_attribute_sets_is_test_marker() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "#[test]\nfn t1() { assert_eq!(1, 1); }\n\
         fn helper() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        is_test_marker_of(&store, "t1"),
        Some(true),
        "#[test] → is_test_marker=true"
    );
    assert_eq!(is_test_marker_of(&store, "helper"), Some(false));
}

#[test]
fn python_test_prefix_sets_is_test_marker() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "tests/test_x.py",
        "def test_one():\n    assert True\n\ndef helper():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(is_test_marker_of(&store, "test_one"), Some(true));
    assert_eq!(is_test_marker_of(&store, "helper"), Some(false));
}

#[test]
fn java_test_annotation_sets_is_test_marker() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "FooTest.java",
        "class FooTest {\n  @Test\n  public void testAdd() {}\n  \
         public void helper() {}\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(is_test_marker_of(&store, "testAdd"), Some(true));
    assert_eq!(is_test_marker_of(&store, "helper"), Some(false));
}

#[test]
fn go_test_naming_sets_is_test_marker() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib_test.go",
        "package lib\nimport \"testing\"\nfunc TestAdd(t *testing.T) {}\n\
         func helper() int { return 1 }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(is_test_marker_of(&store, "TestAdd"), Some(true));
    assert_eq!(is_test_marker_of(&store, "helper"), Some(false));
}

#[test]
fn ruby_test_naming_sets_is_test_marker() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib_test.rb",
        "class C\n  def test_add\n    assert true\n  end\n  def helper\n  end\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(is_test_marker_of(&store, "test_add"), Some(true));
    assert_eq!(is_test_marker_of(&store, "helper"), Some(false));
}
