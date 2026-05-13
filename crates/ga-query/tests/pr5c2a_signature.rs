//! v1.3 PR5c2a — per-lang `extract_modifiers` + `extract_params` for Rust +
//! Python (S-003 AS-007 + AS-008 closure).
//!
//! Spec: spec
//! - AS-007: Rust signature round-trip (params + return_type + modifiers + arity + is_async)
//! - AS-008: Python partial signature with empty-type sentinel
//!
//! PR5c2a ships Rust + Python. PR5c2b will add TS / JS / Go / Java / Kotlin / C# / Ruby.

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

fn modifiers_of(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.modifiers");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(v) = row.into_iter().next() {
            return match v {
                lbug::Value::List(_, items) => items
                    .into_iter()
                    .filter_map(|x| {
                        if let lbug::Value::String(s) = x {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => Vec::new(),
            };
        }
    }
    Vec::new()
}

fn param_count_of(store: &Store, name: &str) -> Option<i64> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN size(s.params)");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Some(n);
        }
    }
    None
}

fn param_names_of(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) UNWIND s.params AS p RETURN p.name");
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.push(s);
        }
    }
    out
}

fn param_types_of(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) UNWIND s.params AS p RETURN p.type");
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        match row.into_iter().next() {
            Some(lbug::Value::String(s)) => out.push(s),
            Some(lbug::Value::Null(_)) => out.push(String::new()),
            _ => {}
        }
    }
    out
}

#[test]
fn rust_modifiers_pub_async() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "pub async fn server() -> i32 { 1 }\n\
         pub fn ordinary() -> i32 { 1 }\n\
         async fn private_async() -> i32 { 1 }\n\
         fn plain() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let m = modifiers_of(&store, "server");
    assert!(m.contains(&"pub".to_string()), "expected pub in {m:?}");
    assert!(m.contains(&"async".to_string()), "expected async in {m:?}");
    assert!(modifiers_of(&store, "ordinary").contains(&"pub".to_string()));
    assert!(modifiers_of(&store, "private_async").contains(&"async".to_string()));
    assert!(
        modifiers_of(&store, "plain").is_empty(),
        "plain has no modifiers"
    );
}

#[test]
fn rust_params_extracted_with_self() {
    // AS-007 partial: Rust `into_symbol(self, file: impl Into<String>)` yields
    // 2 params with names "self" and "file".
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "struct Foo;\n\
         impl Foo {\n\
            fn into_symbol(self, file: String) -> Foo { self }\n\
         }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "into_symbol"), Some(2));
    let names = param_names_of(&store, "into_symbol");
    assert!(names.contains(&"self".to_string()), "names={names:?}");
    assert!(names.contains(&"file".to_string()), "names={names:?}");
    let types = param_types_of(&store, "into_symbol");
    assert!(
        types.iter().any(|t| t.contains("String")),
        "expected one type containing String, got {types:?}"
    );
}

#[test]
fn rust_arity_matches_params_size() {
    // AT-002 audit (tightened by PR5c2a): arity == size(params).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "fn three(a: i32, b: i32, c: i32) -> i32 { a + b + c }\n\
         fn nullary() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "three"), Some(3));
    assert_eq!(param_count_of(&store, "nullary"), Some(0));
}

#[test]
fn python_unannotated_params_have_empty_type_sentinel() {
    // AS-008: `def foo(x, y, z=10)` → params with empty type sentinels.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo(x, y, z=10):\n    return x + y\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "foo"), Some(3));
    let names = param_names_of(&store, "foo");
    assert_eq!(
        names,
        vec!["x".to_string(), "y".to_string(), "z".to_string()]
    );
    let types = param_types_of(&store, "foo");
    // Tools-C2 sentinel — every type empty for unannotated Python.
    assert!(
        types.iter().all(|t| t.is_empty()),
        "AS-008: all types must be empty sentinel, got {types:?}"
    );
}

#[test]
fn python_typed_params_have_type_strings() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def add(a: int, b: int) -> int:\n    return a + b\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "add"), Some(2));
    let types = param_types_of(&store, "add");
    assert!(
        types.iter().filter(|t| *t == "int").count() == 2,
        "expected both params type=int, got {types:?}"
    );
}

#[test]
fn python_class_method_self_counted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "class K:\n    def m(self, x):\n        return x\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "m"), Some(2));
    let names = param_names_of(&store, "m");
    assert_eq!(names, vec!["self".to_string(), "x".to_string()]);
}
