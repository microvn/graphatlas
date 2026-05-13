//! B1 — Rust IMPORTS_NAMED parity.
//!
//! Pre-B1: Rust parser did not override `extract_imported_names` /
//! `extract_imported_aliases`, and `resolve_import_path` returned `None`
//! for Rust → 0 IMPORTS_NAMED rows for any Rust fixture. This left
//! `MATCH (f:File)-[:IMPORTS_NAMED]->(t:Symbol)` (used by ga_hubs since
//! Gap 7 / Fix A) as a no-op for regex / tokio / axum / kotlinx etc.
//!
//! B1 closes the gap with two changes:
//!   1. Parser: walk `use_declaration` and pull names + aliases from
//!      `scoped_identifier`, `use_as_clause`, `scoped_use_list/use_list`,
//!      `use_wildcard` (returns no name), and bare `identifier`.
//!   2. Indexer: when `pi.src_lang == Lang::Rust` and the path resolver
//!      returns None, fall back to repo-wide `symbol_by_name` lookup so
//!      each imported name maps to a single Symbol id (first-write-wins,
//!      matches Tools-C7 policy).
//!
//! Universal-truth: `use foo::Bar` IS a named import — symbol-level,
//! deterministic. No tuning, no fixture-specific heuristic.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::collections::HashSet;
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

fn imports_named_for_src(store: &Store, src: &str) -> Vec<(String, String)> {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (f:File {{path: '{src}'}})-[r:IMPORTS_NAMED]->(s:Symbol) \
         RETURN s.name, r.alias ORDER BY s.name, r.alias"
    );
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        let mut it = row.into_iter();
        let name = match it.next() {
            Some(lbug::Value::String(s)) => s,
            _ => continue,
        };
        let alias = match it.next() {
            Some(lbug::Value::String(s)) => s,
            Some(lbug::Value::Null(_)) => String::new(),
            _ => String::new(),
        };
        out.push((name, alias));
    }
    out
}

#[test]
fn rust_simple_use_emits_imports_named() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.rs",
        "pub struct Foo;\nimpl Foo { pub fn run(&self) {} }\n",
    );
    write_file(
        repo.path(),
        "b.rs",
        "use crate::a::Foo;\npub fn build() -> Foo { Foo }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    assert!(
        rows.iter().any(|(n, a)| n == "Foo" && a.is_empty()),
        "expected (Foo, '') in {rows:?}"
    );
}

#[test]
fn rust_aliased_use_records_alias() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.rs", "pub struct Foo;\n");
    write_file(
        repo.path(),
        "b.rs",
        "use crate::a::Foo as F;\npub fn build() -> F { F }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    assert!(
        rows.iter().any(|(n, a)| n == "Foo" && a == "F"),
        "expected (Foo, 'F') in {rows:?}"
    );
}

#[test]
fn rust_use_list_emits_one_row_per_name() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.rs",
        "pub struct Foo;\npub struct Bar;\npub struct Baz;\n",
    );
    write_file(
        repo.path(),
        "b.rs",
        "use crate::a::{Foo, Bar, Baz};\n\
         pub fn build() -> (Foo, Bar, Baz) { (Foo, Bar, Baz) }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    let set: HashSet<String> = rows.iter().map(|(n, _)| n.clone()).collect();
    for expected in ["Foo", "Bar", "Baz"] {
        assert!(set.contains(expected), "expected {expected} in {rows:?}");
    }
}

#[test]
fn rust_use_list_with_alias_records_alias_only_for_aliased_item() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.rs", "pub struct Foo;\npub struct Bar;\n");
    write_file(
        repo.path(),
        "b.rs",
        "use crate::a::{Foo, Bar as B};\npub fn build() -> (Foo, B) { (Foo, B) }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    assert!(
        rows.iter().any(|(n, a)| n == "Foo" && a.is_empty()),
        "expected (Foo, '') in {rows:?}"
    );
    assert!(
        rows.iter().any(|(n, a)| n == "Bar" && a == "B"),
        "expected (Bar, 'B') in {rows:?}"
    );
}

#[test]
fn rust_use_wildcard_emits_no_imports_named() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.rs", "pub struct Foo;\npub struct Bar;\n");
    write_file(
        repo.path(),
        "b.rs",
        "use crate::a::*;\npub fn use_them() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    // Wildcard binds nothing nameable → 0 IMPORTS_NAMED rows.
    assert!(rows.is_empty(), "wildcard must not emit; got {rows:?}");
}

#[test]
fn rust_unresolved_external_use_drops_silently() {
    // `use std::collections::HashMap;` — HashMap is not indexed (stdlib).
    // Must not panic, must not emit, must not block other uses.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.rs", "pub struct Foo;\n");
    write_file(
        repo.path(),
        "b.rs",
        "use std::collections::HashMap;\nuse crate::a::Foo;\n\
         pub fn run() -> Foo { let _: HashMap<u8, u8> = Default::default(); Foo }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.rs");
    // HashMap not indexed → drops; Foo still resolves.
    assert!(
        rows.iter().any(|(n, _)| n == "Foo"),
        "Foo must resolve even when HashMap drops; got {rows:?}"
    );
    assert!(
        !rows.iter().any(|(n, _)| n == "HashMap"),
        "HashMap is external and must not appear; got {rows:?}"
    );
}

#[test]
fn rust_self_use_does_not_create_self_edge() {
    // `use self::foo::Bar` referring to a symbol in the same file should
    // not produce an IMPORTS_NAMED edge from f → s where s.file == f.path.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.rs",
        "pub struct Foo;\npub use self::Foo as ReFoo;\n",
    );
    write_file(repo.path(), "b.rs", "use crate::a::Foo;\n");
    let (_t, store) = index_repo(repo.path());
    // Edge from a.rs into a.rs's own Foo is a self-edge — must be skipped.
    let conn = store.connection().unwrap();
    let q = "MATCH (f:File)-[:IMPORTS_NAMED]->(s:Symbol) \
             WHERE f.path = s.file RETURN count(*)";
    let rs = conn.query(q).unwrap();
    let mut found_zero = false;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, 0, "self IMPORTS_NAMED edge leaked");
            found_zero = true;
        }
    }
    assert!(found_zero, "count query returned no row");
}

#[test]
fn rust_bare_use_identifier_drops_when_unresolved() {
    // `use foo;` — bound name is `foo`. If no Symbol "foo" is indexed,
    // drop silently (no panic, no synthetic edge).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.rs",
        "use unknown_module;\npub struct Foo;\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "a.rs");
    assert!(
        !rows.iter().any(|(n, _)| n == "unknown_module"),
        "unresolved bare use must drop; got {rows:?}"
    );
}
