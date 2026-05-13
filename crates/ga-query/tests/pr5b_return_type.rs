//! v1.3 PR5b — `Symbol.return_type` per-language extraction (S-003 partial).
//!
//! Spec: spec, S-003
//! AS-007 (return_type assertion) + AS-008 (empty sentinel for unannotated
//! Python).
//!
//! Per-lang AST field map:
//! - Rust: `function_item.return_type` (text after `->`, no leading arrow)
//! - Python: `function_definition.return_type` (text after `->`)
//! - TS: `function_declaration.return_type` / `method_definition.return_type`
//!       contains a `type_annotation` whose first token is `:` — strip leading `:`
//! - Go: `function_declaration.result` / `method_declaration.result`
//! - Java: `method_declaration.type`
//! - Kotlin: `function_declaration` — return type is positional (after params,
//!           after `:`); use heuristic
//! - C#: `method_declaration.returns` (tree-sitter-c-sharp specific)
//! - JS: no static return types — empty sentinel
//! - Ruby: no static return types — empty sentinel
//!
//! Tools-C2 — empty `''` is the unknown sentinel. UC consumers tolerate empty.

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
        // Tools-C2: row exists → return value as-is. lbug parses CSV `""`
        // as NULL on STRING cols; consumers tolerate Null ≡ '' empty sentinel.
        return match row.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            Some(lbug::Value::Null(_)) => Some(String::new()),
            _ => Some(String::new()),
        };
    }
    None
}

#[test]
fn rust_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "fn add(a: i32, b: i32) -> i32 { a + b }\n\
         fn nullary() {}\n\
         fn into_symbol(self, file: String) -> Symbol { todo!() }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "add"), Some("i32".to_string()));
    assert_eq!(
        return_type_of(&store, "into_symbol"),
        Some("Symbol".to_string()),
        "AS-007 Rust into_symbol return_type = Symbol"
    );
    assert_eq!(
        return_type_of(&store, "nullary"),
        Some("".to_string()),
        "no `->` arrow → empty sentinel (Tools-C2)"
    );
}

#[test]
fn python_typed_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def typed(x: int) -> str:\n    return str(x)\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "typed"), Some("str".to_string()));
}

#[test]
fn python_unannotated_return_type_empty_sentinel() {
    // AS-008 sentinel: `def foo(x, y, z=10): return x + y` → return_type == ''
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(x, y, z=10):\n    return x + y\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        return_type_of(&store, "foo"),
        Some("".to_string()),
        "AS-008 Tools-C2: unannotated Python → empty sentinel"
    );
}

#[test]
fn typescript_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.ts",
        "function add(a: number, b: number): number { return a + b; }\n\
         function untyped(x) { return x; }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "add"), Some("number".to_string()));
}

#[test]
fn go_single_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.go",
        "package lib\nfunc Add(a int, b int) int { return a + b }\n\
         func Nullary() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "Add"), Some("int".to_string()));
    assert_eq!(return_type_of(&store, "Nullary"), Some("".to_string()));
}

#[test]
fn java_return_type_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.java",
        "class Foo {\n  int add(int a, int b) { return a + b; }\n  \
         void nullary() {}\n  String greet() { return \"hi\"; }\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "add"), Some("int".to_string()));
    assert_eq!(return_type_of(&store, "greet"), Some("String".to_string()));
    assert_eq!(
        return_type_of(&store, "nullary"),
        Some("void".to_string()),
        "Java void is explicit — not the empty sentinel"
    );
}

#[test]
fn ruby_no_static_return_type_empty() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.rb",
        "class C\n  def add(a, b)\n    a + b\n  end\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        return_type_of(&store, "add"),
        Some("".to_string()),
        "Ruby has no static return types → empty sentinel"
    );
}

#[test]
fn javascript_no_static_return_type_empty() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.js",
        "function add(a, b) { return a + b; }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(
        return_type_of(&store, "add"),
        Some("".to_string()),
        "JS has no static return types → empty sentinel"
    );
}

#[test]
fn classes_have_no_return_type() {
    // Non-function symbols have empty return_type (DDL DEFAULT).
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "lib.py", "class K:\n    pass\n");
    let (_t, store) = index_repo(repo.path());
    assert_eq!(return_type_of(&store, "K"), Some("".to_string()));
}
