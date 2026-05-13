//! Gap 4 — Java/Kotlin/C# annotation extraction → DECORATES edges.
//!
//! Spec: AS-016 — `DECORATES` edge applies to Python decorators AND
//! Java/Kotlin/C# annotations. PR8 wired the indexer DECORATES emission
//! for `SymbolAttribute::Decorator/Annotation`, but only Python parser
//! populates Decorator. This gap closes the JVM/CLR side.
//!
//! Per AS-010 (PR3), `is_override` boolean denormalizes from
//! `SymbolAttribute::Override`. PR3 wired the mapping; this gap finally
//! makes the bool fire for Java `@Override` (currently always false).

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

fn is_override_of(store: &Store, name: &str) -> Option<bool> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.is_override");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            return Some(b);
        }
    }
    None
}

#[test]
fn java_override_annotation_sets_is_override_bool() {
    // AS-010: @Override → is_override = true.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.java",
        "class Foo {\n  @Override\n  public String toString() { return \"foo\"; }\n  \
         public void plain() {}\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        is_override_of(&store, "toString"),
        Some(true),
        "@Override-annotated method must have is_override=true"
    );
    assert_eq!(
        is_override_of(&store, "plain"),
        Some(false),
        "non-annotated method must have is_override=false"
    );
}

#[test]
fn kotlin_override_modifier_sets_is_override_bool() {
    // Kotlin `override fun` is a modifier keyword. tree-sitter-kotlin-ng
    // doesn't support `open class` body well, so test top-level fn with
    // override marker (atomistic syntactic verification).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.kt",
        "package x\noverride fun greet(): String { return \"hi\" }\nfun plain() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let v = is_override_of(&store, "greet");
    assert_eq!(
        v,
        Some(true),
        "Kotlin override fun greet → is_override=true; got {v:?}"
    );
    assert_eq!(
        is_override_of(&store, "plain"),
        Some(false),
        "non-override fn must have is_override=false"
    );
}

#[test]
fn csharp_override_modifier_sets_is_override_bool() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.cs",
        "class Base { public virtual string Greet() { return \"base\"; } }\n\
         class Child : Base { public override string Greet() { return \"child\"; } }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {name: 'Greet'}) RETURN s.is_override")
        .unwrap();
    let mut any_true = false;
    let mut count = 0;
    for row in rs {
        count += 1;
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            if b {
                any_true = true;
            }
        }
    }
    assert!(count >= 1, "expected ≥1 Greet symbol");
    assert!(
        any_true,
        "C# Child.Greet override must have is_override=true (got {count} symbol(s))"
    );
}
