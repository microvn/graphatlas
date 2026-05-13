//! KG-9 — CONTAINS edge emission (class → method).
//!
//! Regression: `crates/ga-query/src/indexer.rs:85-100` dropped the
//! `sym.enclosing` value while building SymbolRow. Parser already tracked
//! it via `ga_parser::walker::walk_node` (walker.rs:33-53); indexer just
//! ignored the signal. CONTAINS table was created in schema since M2
//! (`ga_index::schema:22`) but stayed empty forever.
//!
//! Direction: CONTAINS points FROM class TO member (method / field / inner
//! class) — matches rust-poc/src/main.rs:1935-1962 ("class_symbol →
//! method_symbol (enclosing_class relationship)") and enables
//! `<-[:CONTAINS]-(cls)-[:CONTAINS]->(sib)` reverse-forward traversal
//! (rust-poc:2217-2227) for sibling-method blast radius.

use ga_index::Store;
use ga_query::indexer::build_index;
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

/// Member names CONTAINS'd by `class_name` in the given fixture.
fn members_of(store: &Store, class_name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let cypher = format!(
        "MATCH (cls:Symbol {{name: '{class_name}'}})-[:CONTAINS]->(m:Symbol) \
         RETURN m.name"
    );
    let rs = conn.query(&cypher).unwrap();
    let mut out: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(n)) => Some(n),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[test]
fn contains_emitted_for_python_class_methods() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("mod.py"),
        "class Foo:\n    def bar(self):\n        pass\n\n    def baz(self):\n        pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let members = members_of(&store, "Foo");
    assert_eq!(members, vec!["bar".to_string(), "baz".to_string()]);
}

#[test]
fn contains_not_emitted_for_top_level_function() {
    // Top-level function has no enclosing class → must not create CONTAINS.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("free.py"), "def standalone():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH ()-[r:CONTAINS]->() RETURN count(r)")
        .unwrap();
    let count = rs
        .into_iter()
        .next()
        .and_then(|r| match r.into_iter().next() {
            Some(lbug::Value::Int64(n)) => Some(n),
            _ => None,
        })
        .unwrap_or(-1);
    assert_eq!(count, 0, "no class present — expected zero CONTAINS edges");
}

#[test]
fn contains_class_does_not_contain_itself() {
    // Safety: the class symbol itself must not appear as its own member.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("c.py"),
        "class Foo:\n    def bar(self):\n        pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let members = members_of(&store, "Foo");
    assert!(
        !members.contains(&"Foo".to_string()),
        "class must not CONTAIN itself: {members:?}"
    );
}

#[test]
fn contains_edge_is_direction_class_to_method() {
    // Direction check: pattern (cls)-[:CONTAINS]->(m) should find method;
    // reverse pattern (m)-[:CONTAINS]->(cls) should be empty.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("d.py"),
        "class Foo:\n    def bar(self):\n        pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let conn = store.connection().unwrap();
    let forward = conn
        .query(
            "MATCH (cls:Symbol {name: 'Foo'})-[:CONTAINS]->(m:Symbol {name: 'bar'}) \
             RETURN count(m)",
        )
        .unwrap();
    let fwd_count = forward
        .into_iter()
        .next()
        .and_then(|r| match r.into_iter().next() {
            Some(lbug::Value::Int64(n)) => Some(n),
            _ => None,
        })
        .unwrap_or(-1);
    assert_eq!(fwd_count, 1, "forward (class → method) edge expected");

    let reverse = conn
        .query(
            "MATCH (m:Symbol {name: 'bar'})-[:CONTAINS]->(cls:Symbol {name: 'Foo'}) \
             RETURN count(cls)",
        )
        .unwrap();
    let rev_count = reverse
        .into_iter()
        .next()
        .and_then(|r| match r.into_iter().next() {
            Some(lbug::Value::Int64(n)) => Some(n),
            _ => None,
        })
        .unwrap_or(-1);
    assert_eq!(
        rev_count, 0,
        "reverse edge must NOT exist (breaks semantic)"
    );
}
