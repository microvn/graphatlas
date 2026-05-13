//! Tools S-002 cluster C — AS-005 end-to-end: TypeScript arrow/closure
//! callees surface as callees of the enclosing function, not dropped.

use ga_index::Store;
use ga_query::{callees, indexer::build_index};
use std::fs;
use tempfile::TempDir;

#[test]
fn process_users_calls_validate_user_via_map() {
    // AS-005 literal: .map(u => validateUser(u)) must produce
    // CALLS(processUsers → validateUser).
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("users.ts"),
        "export function processUsers(users: unknown[]) {\n\
         \u{20}   return users.map(u => validateUser(u));\n\
         }\n\
         function validateUser(u: unknown) { return u; }\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "processUsers", None).unwrap();
    assert!(resp.meta.symbol_found, "processUsers must be indexed");
    let names: Vec<&str> = resp.callees.iter().map(|c| c.symbol.as_str()).collect();
    assert!(
        names.contains(&"validateUser"),
        "arrow callee must resolve to outer fn: {names:?}"
    );
}

#[test]
fn nested_arrow_chain_still_attributes_all_calls() {
    // .filter(...).map(...) — two anonymous arrows, calls inside each should
    // attribute to processUsers.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("users.ts"),
        "function isActive(u: any): boolean { return !!u; }\n\
         function validateUser(u: any) { return u; }\n\
         export function processUsers(users: any[]) {\n\
         \u{20}   return users.filter(u => isActive(u)).map(u => validateUser(u));\n\
         }\n",
    )
    .unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "processUsers", None).unwrap();
    let names: std::collections::HashSet<&str> =
        resp.callees.iter().map(|c| c.symbol.as_str()).collect();
    assert!(names.contains("validateUser"), "{names:?}");
    assert!(names.contains("isActive"), "{names:?}");
}
