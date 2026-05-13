//! Gap 3 — Kotlin + C# `Symbol.return_type` extraction.
//!
//! Spec: AS-007 multi-language signature round-trip. PR5b inherited None for
//! Kotlin/C# (parser AST research deferred). This gap closes both langs.

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

fn return_type_of(store: &Store, name: &str) -> Option<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.return_type");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        return match row.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            Some(lbug::Value::Null(_)) => Some(String::new()),
            _ => Some(String::new()),
        };
    }
    None
}

#[test]
fn kotlin_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.kt",
        "package x\nfun do_work(input: Int): Int { return input }\n\
         fun nullary() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rt = return_type_of(&store, "do_work");
    assert!(
        rt.as_deref() == Some("Int"),
        "Kotlin do_work return_type should be `Int`, got {rt:?}"
    );
    let rt_null = return_type_of(&store, "nullary");
    assert_eq!(
        rt_null,
        Some("".to_string()),
        "Kotlin nullary (no `: T` annotation) should be empty sentinel"
    );
}

#[test]
fn csharp_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.cs",
        "class Foo {\n  public int Add(int a, int b) { return a + b; }\n  \
         public void Nullary() {}\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        return_type_of(&store, "Add"),
        Some("int".to_string()),
        "C# Add return_type should be `int`"
    );
    assert_eq!(
        return_type_of(&store, "Nullary"),
        Some("void".to_string()),
        "C# void is explicit"
    );
}
