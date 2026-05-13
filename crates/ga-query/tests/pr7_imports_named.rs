//! v1.3 PR7 — IMPORTS_NAMED emission (S-006 AS-014).
//!
//! Spec: spec, S-006.
//!
//! Then-clause: `import { foo, bar as baz } from "./a"` produces 2 IMPORTS_NAMED
//! rows: (name="foo", alias="") + (name="bar", alias="baz"). Legacy IMPORTS
//! (File→File) also emitted (backward compat per Tools-C7).

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
fn ts_named_import_resolves_to_target_symbols() {
    // AS-014 verbatim — `import { foo, bar as baz } from "./a"` emits 2 rows.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.ts",
        "export function foo() { return 1; }\nexport function bar() { return 2; }\n",
    );
    write_file(
        repo.path(),
        "b.ts",
        "import { foo, bar as baz } from \"./a\";\nexport function use_them() { return foo() + baz(); }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.ts");
    let set: HashSet<(String, String)> = rows.iter().cloned().collect();
    assert!(
        set.contains(&("foo".to_string(), "".to_string())),
        "expected (foo, '') in {rows:?}"
    );
    assert!(
        set.contains(&("bar".to_string(), "baz".to_string())),
        "expected (bar, 'baz') in {rows:?}"
    );
}

#[test]
fn imports_named_co_exists_with_legacy_imports_at_same_line() {
    // AT-003: every IMPORTS_NAMED row has a matching IMPORTS row at same
    // (file, import_line). Backward-compat per Tools-C7 strict-union catch-all.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.ts", "export function foo() { return 1; }\n");
    write_file(
        repo.path(),
        "b.ts",
        "import { foo } from \"./a\";\nexport function go() { return foo(); }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    // IMPORTS_NAMED edge present
    let rs = conn
        .query(
            "MATCH (f:File {path: 'b.ts'})-[r:IMPORTS_NAMED]->(s:Symbol) \
             RETURN r.import_line",
        )
        .unwrap();
    let mut named_lines = Vec::new();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            named_lines.push(n);
        }
    }
    // Legacy IMPORTS (File→File) present at same line
    let rs = conn
        .query(
            "MATCH (f:File {path: 'b.ts'})-[r:IMPORTS]->(:File) \
             RETURN r.import_line",
        )
        .unwrap();
    let mut legacy_lines = Vec::new();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            legacy_lines.push(n);
        }
    }
    assert!(!named_lines.is_empty(), "no IMPORTS_NAMED edge");
    assert!(!legacy_lines.is_empty(), "no legacy IMPORTS edge");
    let named_set: HashSet<i64> = named_lines.iter().copied().collect();
    let legacy_set: HashSet<i64> = legacy_lines.iter().copied().collect();
    assert!(
        !named_set.is_disjoint(&legacy_set),
        "AT-003: IMPORTS_NAMED line ({named_set:?}) must overlap legacy IMPORTS line ({legacy_set:?})"
    );
}

#[test]
fn python_named_import_resolves() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "pkg/__init__.py", "");
    write_file(
        repo.path(),
        "pkg/a.py",
        "def foo():\n    return 1\n\ndef bar():\n    return 2\n",
    );
    write_file(
        repo.path(),
        "pkg/b.py",
        "from pkg.a import foo, bar\n\ndef use():\n    return foo() + bar()\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "pkg/b.py");
    let names: HashSet<String> = rows.iter().map(|(n, _)| n.clone()).collect();
    assert!(names.contains("foo"), "rows={rows:?}");
    assert!(names.contains("bar"), "rows={rows:?}");
}

#[test]
fn external_imports_drop_silently() {
    // Importing from a file NOT in the indexed set → no IMPORTS_NAMED edges
    // (target Symbol can't be resolved). Tools-C12.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.ts",
        "import { somefn } from \"some-package\";\nexport function go() { return somefn(); }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "main.ts");
    assert!(
        rows.is_empty(),
        "external imports should drop silently, got {rows:?}"
    );
}

#[test]
fn imports_named_only_for_resolved_symbols() {
    // Importing a name that doesn't exist in target file → no row for that
    // name. Other names in same import statement still resolve.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.ts",
        "export function foo() { return 1; }\n", // no `bar` exported
    );
    write_file(
        repo.path(),
        "b.ts",
        "import { foo, bar } from \"./a\";\nexport function use() { return foo(); }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let rows = imports_named_for_src(&store, "b.ts");
    let names: HashSet<String> = rows.iter().map(|(n, _)| n.clone()).collect();
    assert!(names.contains("foo"), "foo should resolve, rows={rows:?}");
    assert!(
        !names.contains("bar"),
        "bar doesn't exist in a.ts; no IMPORTS_NAMED edge expected, rows={rows:?}"
    );
}
