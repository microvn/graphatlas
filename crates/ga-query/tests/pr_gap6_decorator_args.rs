//! Gap 6 — AS-016 decorator_args extraction.
//!
//! Spec: AS-016 expects `r.decorator_args` populated with the raw argument
//! list source text (e.g. `('/users', methods=['GET'])`). Spec also mentions
//! JSON-serialized form `'["/users",{"methods":["GET"]}]'` as the ideal —
//! that requires per-lang argument-AST → JSON conversion which is bigger
//! work. v1.3 ships **raw source text** of the argument list which preserves
//! UC-relevant info (route path, methods list) without per-lang JSON conv.
//!
//! v1.5+: JSON normalization once UC consumers actually need structured args.

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

fn decorator_args_for(store: &Store, target_name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (d:Symbol)-[r:DECORATES]->(t:Symbol {{name: '{target_name}'}}) \
         RETURN r.decorator_args"
    );
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
fn python_no_args_decorator_emits_empty_args() {
    // `@my_decorator` (no parens, no args) — decorator_args should be empty.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def my_decorator(fn):\n    return fn\n\
         \n\
         @my_decorator\n\
         def target():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let args = decorator_args_for(&store, "target");
    assert_eq!(args.len(), 1, "expected 1 DECORATES edge, got {args:?}");
    assert_eq!(args[0], "", "no-arg decorator → empty decorator_args");
}

#[test]
fn python_decorator_with_args_emits_raw_arg_text() {
    // `@app.route('/users', methods=['GET'])` — decorator_args carries
    // raw arg-list source text (paren-stripped).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def route(path, methods=None):\n    def deco(fn):\n        return fn\n    return deco\n\
         \n\
         @route('/users', methods=['GET'])\n\
         def list_users():\n    return []\n",
    );
    let (_t, store) = index_repo(repo.path());
    let args = decorator_args_for(&store, "list_users");
    assert_eq!(args.len(), 1, "expected 1 DECORATES edge, got {args:?}");
    let a = &args[0];
    // Tools-C14 sanitizer strips `'` and `,`. We assert the path component
    // and the methods key remain identifiable.
    assert!(
        a.contains("/users") || a.contains("users"),
        "args must mention the route path; got {a:?}"
    );
    assert!(
        a.contains("methods") || a.contains("GET"),
        "args must mention methods/GET; got {a:?}"
    );
}

#[test]
fn python_simple_string_arg_decorator() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def tag(label):\n    def deco(fn):\n        return fn\n    return deco\n\
         \n\
         @tag('experimental')\n\
         def feature():\n    return 1\n",
    );
    let (_t, store) = index_repo(repo.path());
    let args = decorator_args_for(&store, "feature");
    assert_eq!(args.len(), 1);
    let a = &args[0];
    assert!(
        a.contains("experimental"),
        "single-string arg must round-trip; got {a:?}"
    );
}
