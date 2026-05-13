//! Indexer populates EXTENDS edges — prerequisite for auto-GT H1.

use ga_index::Store;
use ga_query::indexer::build_index;
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
fn python_class_inheritance_emits_extends_edge() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("shapes.py"),
        "class Shape:\n    def area(self): return 0\n\nclass Circle(Shape):\n    def area(self): return 3.14\n\nclass Square(Shape):\n    def area(self): return 1\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.extends_edges >= 2,
        "expect ≥2 EXTENDS edges (Circle→Shape, Square→Shape), got {}",
        stats.extends_edges
    );

    // Query: find what Circle extends.
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (src:Symbol)-[:EXTENDS]->(dst:Symbol) \
             WHERE src.name = 'Circle' RETURN dst.name",
        )
        .unwrap();
    let mut bases = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            bases.push(s);
        }
    }
    assert_eq!(bases, vec!["Shape".to_string()]);
}

#[test]
fn ts_class_inheritance_emits_extends_edge() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("shapes.ts"),
        "export class Shape { area() { return 0; } }\nexport class Circle extends Shape { area() { return 3.14; } }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(stats.extends_edges >= 1, "got {}", stats.extends_edges);
}

#[test]
fn rust_impl_trait_for_struct_emits_extends_edge() {
    // `impl Trait for Struct` → Struct EXTENDS Trait. Verifies impl_item
    // special-case in extends.rs + broadened class_by_name lookup.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("lib.rs"),
        "pub trait Greet {\n    fn hello(&self) -> String;\n}\n\npub struct English;\n\nimpl Greet for English {\n    fn hello(&self) -> String { String::from(\"hi\") }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.extends_edges >= 1,
        "Rust impl must emit EXTENDS: got {}",
        stats.extends_edges
    );
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (src:Symbol)-[:EXTENDS]->(dst:Symbol) \
             WHERE src.name = 'English' RETURN dst.name",
        )
        .unwrap();
    let mut bases = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            bases.push(s);
        }
    }
    assert_eq!(bases, vec!["Greet".to_string()]);
}

#[test]
fn no_inheritance_emits_zero_extends() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "class Lonely:\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.extends_edges, 0);
}

#[test]
fn extends_with_unresolved_base_does_not_emit_edge() {
    // Base class lives outside the repo — indexer drops the edge (same
    // policy as CALLS cross-file: can't emit an edge to a missing node).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "class Local(ExternalBase):\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    // ExternalBase isn't defined anywhere → edge dropped.
    assert_eq!(stats.extends_edges, 0);
}
