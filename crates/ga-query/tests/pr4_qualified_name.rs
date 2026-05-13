//! v1.3 PR4 — `Symbol.qualified_name` per-language population (S-002).
//!
//! Spec: spec, S-002.
//!
//! Scope (PR4a baseline — covers full PR4 user-value):
//! - AS-004 — qualified_name populated per-lang AND stable across rebuild.
//! - AS-005 — Rust collision classes resolve via class-chain (impl receivers
//!   via EnclosingScope::Class) + dedup suffix for overloads/macros.
//! - AS-006 — indexer-side dedup with `#dup<N>` suffix on collision.
//! - AT-001 audit (subset): zero rows have qualified_name = ''.
//!
//! Per-lang format (universal-truth chain via existing EnclosingScope):
//! - Rust:        `{rel}::{enclosing_chain}::{name}`  (`::` separator)
//! - Ruby:        `{rel}::{enclosing_chain}#{name}`   (`#` separator)
//! - Default (Py/TS/JS/Go/Java/Kotlin/C#/etc.): `{rel}::{enclosing_chain}.{name}`
//!
//! Java/Kotlin/C# `(arity)` overload suffix deferred to PR5 (needs signature
//! extraction). Until then, overloaded methods collide on qualified_name and
//! are deduplicated via AS-006 `#dup<N>` suffix → still UNIQUE per AT-001.

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

fn collect_qns(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (s:Symbol {{name: '{name}'}}) RETURN s.qualified_name ORDER BY s.qualified_name"
    );
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.push(s);
        }
    }
    out
}

#[test]
fn as_004_python_qualified_name_uses_dot_separator() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "pkg/mod.py",
        "class C:\n    def m(self):\n        return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let qns = collect_qns(&store, "m");
    assert_eq!(
        qns,
        vec!["pkg/mod.py::C.m".to_string()],
        "Python uses `.` separator + class chain"
    );
}

#[test]
fn as_004_rust_qualified_name_uses_double_colon() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "struct Foo;\nimpl Foo {\n    fn bar(&self) -> i32 { 1 }\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let qns = collect_qns(&store, "bar");
    assert_eq!(qns.len(), 1, "expected 1 row for bar, got {qns:?}");
    let qn = &qns[0];
    assert!(
        qn.starts_with("src/lib.rs::"),
        "Rust qn must start with rel_path::, got {qn}"
    );
    assert!(
        qn.ends_with("::bar"),
        "Rust uses `::` separator before name, got {qn}"
    );
    assert!(
        !qn.contains(".bar"),
        "Rust must NOT use `.` separator, got {qn}"
    );
}

#[test]
fn as_004_ruby_qualified_name_uses_hash_separator() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib/foo.rb",
        "class C\n  def bar\n    1\n  end\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    let qns = collect_qns(&store, "bar");
    assert_eq!(qns.len(), 1, "expected 1 row for bar, got {qns:?}");
    let qn = &qns[0];
    assert!(qn.contains('#'), "Ruby qn must use `#` separator, got {qn}");
    assert!(qn.ends_with("#bar"), "Ruby qn must end with #bar, got {qn}");
}

#[test]
fn as_004_qualified_name_stable_across_line_shift() {
    // Reindex same file with a 5-line prologue prepended. id changes (line
    // moves); qualified_name must be byte-identical.
    let repo_a = TempDir::new().unwrap();
    write_file(
        repo_a.path(),
        "pkg/mod.py",
        "class C:\n    def stable_fn(self):\n        return 1\n",
    );
    let (_t1, store_a) = index_repo(repo_a.path());
    let qn_a = collect_qns(&store_a, "stable_fn");

    let repo_b = TempDir::new().unwrap();
    let prologue = "# ".to_string() + &"x ".repeat(60) + "\n# y\n# z\n# w\n# v\n";
    write_file(
        repo_b.path(),
        "pkg/mod.py",
        &(prologue + "class C:\n    def stable_fn(self):\n        return 1\n"),
    );
    let (_t2, store_b) = index_repo(repo_b.path());
    let qn_b = collect_qns(&store_b, "stable_fn");

    assert_eq!(
        qn_a, qn_b,
        "qualified_name must be stable across line shift"
    );
    assert_eq!(qn_a.len(), 1);
}

#[test]
fn as_006_collision_dedup_appends_dup_suffix() {
    // Two same-named functions in the same enclosing → both pushed to walker
    // (Symbol.id distinguishes by line). qualified_name pre-dedup collides;
    // post-dedup, second instance gets `#dup1`.
    //
    // Mimics AS-005 class (d) macro `define_handler!(foo) ×3` and class (e)
    // overloaded `fn add(i32) / fn add(f64)` — both observable by Python with
    // shadowed defs in same scope (which is what tree-sitter emits).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo():\n    return 1\n\ndef foo():\n    return 2\n\ndef foo():\n    return 3\n",
    );
    let (_t, store) = index_repo(repo.path());
    let qns = collect_qns(&store, "foo");
    assert_eq!(qns.len(), 3, "expected 3 foo symbols, got {qns:?}");
    let unique: std::collections::HashSet<_> = qns.iter().collect();
    assert_eq!(
        unique.len(),
        3,
        "AS-006 dedup must produce 3 UNIQUE qualified_names, got {qns:?}"
    );
    let dup_count = qns.iter().filter(|q| q.contains("#dup")).count();
    assert!(
        dup_count >= 2,
        "expected ≥2 `#dup<N>` suffixed qns, got {qns:?}"
    );
}

#[test]
fn at_001_audit_no_empty_qualified_names() {
    // AT-001 baseline audit: every Symbol row has qualified_name != ''.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.py",
        "def alpha():\n    return 1\nclass K:\n    def beta(self): return 2\n",
    );
    write_file(
        repo.path(),
        "b.rs",
        "fn gamma() -> i32 { 1 }\nstruct S;\nimpl S { fn delta(&self) -> i32 { 1 } }\n",
    );
    write_file(
        repo.path(),
        "c.go",
        "package c\nfunc Epsilon() int { return 1 }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (s:Symbol) WHERE s.qualified_name = '' AND s.kind <> 'external' RETURN count(s)",
        )
        .unwrap();
    let mut empty = i64::MAX;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            empty = n;
        }
    }
    assert_eq!(
        empty, 0,
        "AT-001: zero non-external rows must have empty qualified_name"
    );
}

#[test]
fn as_005_class_a_rust_impl_separates_by_receiver() {
    // AS-005 class (a) — `impl Display for Foo { fn fmt }` and
    // `impl Display for Bar { fn fmt }` in same file. The receiver type
    // (Foo / Bar) is the EnclosingScope::Class in the parser's view, so
    // qualified_names diverge naturally (no #dup needed).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "struct Foo;\nstruct Bar;\nimpl Foo { fn fmt(&self) {} }\nimpl Bar { fn fmt(&self) {} }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let qns = collect_qns(&store, "fmt");
    assert_eq!(qns.len(), 2, "expected 2 fmt rows, got {qns:?}");
    let unique: std::collections::HashSet<_> = qns.iter().collect();
    assert_eq!(
        unique.len(),
        2,
        "AS-005(a): impl-receiver disambiguation must yield 2 UNIQUE qns, got {qns:?}"
    );
    // One must contain Foo, the other Bar
    assert!(
        qns.iter().any(|q| q.contains("Foo")),
        "missing Foo qn: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("Bar")),
        "missing Bar qn: {qns:?}"
    );
}
