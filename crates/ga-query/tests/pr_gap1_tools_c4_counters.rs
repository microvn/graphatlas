//! Gap 1 — Tools-C4 stats counter exposure.
//!
//! Spec: spec, Tools-C4
//! line 327: "Unresolved import counter (`unresolved_imports_count`) is a
//! per-index metric exposed via stats; same for `unresolved_decorators_count`
//! and `qualified_name_collision_count`."
//!
//! IndexStats grows 3 fields. Counters surface counts already collected as
//! local vars during PR4 (qualified_name dedup), PR7 (IMPORTS_NAMED), PR8
//! (DECORATES).

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> ga_query::indexer::IndexStats {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    let stats = build_index(&store, repo).unwrap();
    store.commit().unwrap();
    stats
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

#[test]
fn qualified_name_collision_counter_surfaces_in_stats() {
    // Force a same-file qualified_name collision (multiple `foo` shadowed defs).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def foo():\n    return 1\n\ndef foo():\n    return 2\n\ndef foo():\n    return 3\n",
    );
    let stats = index_repo(repo.path());
    assert!(
        stats.qualified_name_collision_count >= 2,
        "expected ≥2 collisions for 3 same-named foo defs, got {}",
        stats.qualified_name_collision_count
    );
}

#[test]
fn unresolved_decorators_counter_surfaces_in_stats() {
    // External decorator (stdlib `@functools.lru_cache`) → no DECORATES edge,
    // unresolved counter increments.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "import functools\n\
         \n\
         @functools.lru_cache\n\
         def expensive():\n    return 1\n",
    );
    let stats = index_repo(repo.path());
    assert!(
        stats.unresolved_decorators_count >= 1,
        "expected ≥1 unresolved decorator (functools.lru_cache), got {}",
        stats.unresolved_decorators_count
    );
}

#[test]
fn unresolved_imports_counter_surfaces_when_named_unresolved() {
    // TS `import { non_existent } from "./a"` where a.ts doesn't define
    // non_existent → IMPORTS_NAMED resolution fails for that name.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.ts", "export function foo() { return 1; }\n");
    write_file(
        repo.path(),
        "b.ts",
        "import { foo, bar_does_not_exist } from \"./a\";\nexport function go() { return foo(); }\n",
    );
    let stats = index_repo(repo.path());
    assert!(
        stats.unresolved_imports_count >= 1,
        "expected ≥1 unresolved import (bar_does_not_exist), got {}",
        stats.unresolved_imports_count
    );
}

#[test]
fn no_collisions_no_unresolved_counters_zero() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "main.py", "def hello():\n    return 1\n");
    let stats = index_repo(repo.path());
    assert_eq!(stats.qualified_name_collision_count, 0);
    assert_eq!(stats.unresolved_decorators_count, 0);
    assert_eq!(stats.unresolved_imports_count, 0);
}
