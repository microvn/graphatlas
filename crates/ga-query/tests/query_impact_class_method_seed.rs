//! infra:S-004 — class-method seed resolution.
//!
//! AS-011: Python `User.set_password` → split on `.` → resolve via enclosing
//! AS-012: Rust `Router::new` → split on `::`
//! AS-013: Ambiguous `save` → file hint picks correct one (Tools-C11)
//! AS-014: `Nonexistent.method` → error with suggestions
//!
//! Fix target: `graphatlas-tools.md:307,323` audit finding. Django 4/6 stuck
//! tasks return `actual_files=0` because Cypher `WHERE s.name = <seed>` does
//! not match qualified names.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
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

/// AS-011 happy path — Python `User.set_password` qualified seed resolves.
#[test]
fn qualified_python_class_method_resolves() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("models.py"),
        "class User:\n\
         \x20   def set_password(self, raw):\n\
         \x20       hasher.hash(raw)\n",
    );
    write(&repo.join("hasher.py"), "def hash(raw):\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("User.set_password".into()),
            file: Some("models.py".into()),
            ..Default::default()
        },
    )
    .expect("qualified seed must resolve, not error");

    assert!(
        !resp.impacted_files.is_empty(),
        "AS-011: qualified seed User.set_password should return non-empty \
         impacted_files (at least seed file at depth=0); got empty"
    );
    let paths: Vec<&str> = resp
        .impacted_files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(
        paths.contains(&"models.py"),
        "AS-011: seed file models.py missing from impacted_files; got {:?}",
        paths
    );
}

/// AS-012 happy path — Rust `Router::new` qualified seed (double-colon) resolves.
#[test]
fn qualified_rust_double_colon_resolves() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/lib.rs"),
        "pub struct Router;\n\
         impl Router {\n\
         \x20   pub fn new() -> Self { Router }\n\
         }\n",
    );
    write(
        &repo.join("src/main.rs"),
        "use crate::Router;\n\
         fn bootstrap() {\n\
         \x20   let r = Router::new();\n\
         }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("Router::new".into()),
            file: Some("src/lib.rs".into()),
            ..Default::default()
        },
    )
    .expect("qualified seed with :: must resolve");

    assert!(
        !resp.impacted_files.is_empty(),
        "AS-012: qualified seed Router::new should resolve via :: split; got empty"
    );
}

/// AS-014 error path — unresolvable qualified seed returns InvalidParams.
#[test]
fn qualified_not_found_returns_invalid_params_with_suggestions() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("models.py"),
        "class User:\n\
         \x20   def set_password(self):\n\
         \x20       pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let err = impact(
        &store,
        &ImpactRequest {
            symbol: Some("NonexistentClass.missing_method".into()),
            ..Default::default()
        },
    )
    .expect_err("AS-014: unresolvable qualified seed must return error, not empty response");

    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("not found"),
        "AS-014: error message must indicate 'not found'; got: {}",
        msg
    );
    // Suggestions should include at least one real symbol from index.
    assert!(
        msg.to_lowercase().contains("set_password") || msg.to_lowercase().contains("user"),
        "AS-014: error message should include suggestions (top-3 Levenshtein); got: {}",
        msg
    );
}

/// AS-013 — ambiguous unqualified seed + file hint → file_hint fallback
/// narrows resolution (Tools-C11 polymorphic confidence pattern). Three
/// classes define `save`; seed=`save` with file hint for A.py must surface
/// A.py's save first (via seed.rs file_hint fallback path).
///
/// Phase A review [H-3]: earlier `.build-checklist` claimed AS-013 covered,
/// but no explicit test existed.
#[test]
fn file_hint_narrows_ambiguous_seed_to_correct_class() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "class A:\n    def save(self):\n        a_helper()\n\ndef a_helper():\n    pass\n",
    );
    write(
        &repo.join("b.py"),
        "class B:\n    def save(self):\n        b_helper()\n\ndef b_helper():\n    pass\n",
    );
    write(
        &repo.join("c.py"),
        "class C:\n    def save(self):\n        c_helper()\n\ndef c_helper():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // Seed `save` with file hint pointing at a.py — per Tools-C11 the
    // hint anchors resolution to A.save specifically.
    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("save".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .expect("AS-013: ambiguous seed + file hint must resolve, not error");

    // `impacted_files` must include a.py (seed file at depth 0). If
    // file_hint fallback is broken, resolver would pick a random B/C::save
    // or fail entirely.
    let paths: Vec<&str> = resp
        .impacted_files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(
        paths.contains(&"a.py"),
        "AS-013: file hint a.py must anchor resolution to A.save; \
         impacted_files got {paths:?}"
    );
}

/// M-3 review: `crate::Type` — well-known root should skip CONTAINS
/// traversal and resolve on the last segment directly.
#[test]
fn well_known_root_crate_resolves_on_last_segment() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/lib.rs"),
        "pub struct MyType;\nimpl MyType {\n    pub fn work() {}\n}\n",
    );
    write(
        &repo.join("src/main.rs"),
        "use crate::MyType;\nfn bootstrap() {\n    MyType::work();\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("crate::MyType".into()),
            file: Some("src/lib.rs".into()),
            ..Default::default()
        },
    )
    .expect("M-3: `crate::Type` must resolve via well-known-root skip");

    assert!(
        !resp.impacted_files.is_empty(),
        "M-3: crate::MyType should resolve on last segment (MyType), not \
         attempt CONTAINS with enclosing=crate"
    );
}

/// Regression guard — unqualified seed (existing behavior) still works.
#[test]
fn unqualified_seed_still_resolves_regression_guard() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def process():\n    helper()\n\ndef helper():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("helper".into()),
            ..Default::default()
        },
    )
    .expect("unqualified seed must keep working (regression guard)");

    assert!(
        !resp.impacted_files.is_empty(),
        "Regression: unqualified seed 'helper' should still resolve"
    );
}
