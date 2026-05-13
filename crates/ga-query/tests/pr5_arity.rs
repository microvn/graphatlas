//! v1.3 PR5a — `Symbol.arity` per-language extraction (S-003 partial).
//!
//! Spec: spec, S-003
//! AS-007 / AS-008 (arity assertion subset).
//!
//! PR5 split status (this session = PR5a):
//! - [x] arity scalar across 9 wired langs (this PR5a)
//! - [ ] return_type STRING per-lang (PR5b — next session)
//! - [ ] params STRUCT[] + modifiers STRING[] composites (PR5c — kuzu#6045
//!       trap; needs Tools-C13 full-row Path G CSV emission)
//!
//! AT-002 audit (subset): once params lands, `arity == size(params)` per row.
//! For now: arity = count of formal-parameter named children at parser time.

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

fn arity_of(store: &Store, name: &str) -> Option<i64> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.arity");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Some(n);
        }
    }
    None
}

#[test]
fn rust_arity_counts_self_and_explicit_params() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "struct S;\nimpl S {\n    fn two(&self, x: i32) -> i32 { x }\n}\n\
         fn three(a: i32, b: i32, c: i32) -> i32 { a + b + c }\nfn nullary() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        arity_of(&store, "two"),
        Some(2),
        "Rust &self + 1 param = arity 2"
    );
    assert_eq!(arity_of(&store, "three"), Some(3));
    assert_eq!(arity_of(&store, "nullary"), Some(0));
}

#[test]
fn python_arity_counts_self_and_default_params() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(x, y, z=10):\n    return x + y\n\
         def nullary():\n    pass\n\
         class C:\n    def m(self, a, b):\n        return a\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        arity_of(&store, "foo"),
        Some(3),
        "AS-008 python def foo(x,y,z=10) arity = 3"
    );
    assert_eq!(arity_of(&store, "nullary"), Some(0));
    assert_eq!(
        arity_of(&store, "m"),
        Some(3),
        "Python self + a + b = arity 3"
    );
}

#[test]
fn typescript_arity_counts_formal_parameters() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.ts",
        "function add(a: number, b: number): number { return a + b; }\n\
         function nullary(): void {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(arity_of(&store, "add"), Some(2));
    assert_eq!(arity_of(&store, "nullary"), Some(0));
}

#[test]
fn go_arity_counts_parameter_list() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.go",
        "package lib\n\
         func Add(a int, b int) int { return a + b }\n\
         func Nullary() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(arity_of(&store, "Add"), Some(2));
    assert_eq!(arity_of(&store, "Nullary"), Some(0));
}

#[test]
fn java_arity_counts_formal_parameters() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.java",
        "class Foo {\n  int add(int a, int b) { return a + b; }\n  void nullary() {}\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(arity_of(&store, "add"), Some(2));
    assert_eq!(arity_of(&store, "nullary"), Some(0));
}

#[test]
fn ruby_arity_counts_method_parameters() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.rb",
        "class C\n  def add(a, b)\n    a + b\n  end\n  def nullary\n  end\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(arity_of(&store, "add"), Some(2));
    assert_eq!(arity_of(&store, "nullary"), Some(0));
}

#[test]
fn at_002_audit_arity_matches_explicit_param_count() {
    // AT-002 baseline (subset): arity matches the count visible from source.
    // Once params STRUCT[] lands (PR5c), the audit will tighten to
    // `arity == size(params)`. For now, this test pins per-lang counts so
    // PR5b/PR5c can extend without regressing arity.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def f(x, y, z): return x\n");
    write_file(
        repo.path(),
        "b.rs",
        "fn g(a: i32, b: i32, c: i32, d: i32) -> i32 { a }\n",
    );
    write_file(
        repo.path(),
        "c.go",
        "package c\nfunc H(p, q string) string { return p }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(arity_of(&store, "f"), Some(3));
    assert_eq!(arity_of(&store, "g"), Some(4));
    assert_eq!(arity_of(&store, "H"), Some(2));
}

#[test]
fn classes_and_other_non_function_symbols_arity_default() {
    // Class/struct/trait symbols don't have parameters → arity stays at
    // DDL DEFAULT -1 (Tools-C2 unknown sentinel) so downstream filters
    // distinguish "function with 0 params" from "non-function".
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "lib.py", "class K:\n    pass\n");
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        arity_of(&store, "K"),
        Some(-1),
        "non-function symbols keep DDL DEFAULT -1 (Tools-C2)"
    );
}
