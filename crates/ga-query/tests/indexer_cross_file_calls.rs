//! Cross-file CALLS resolution.
//!
//! Regression: indexer.rs:183 — CALLS resolution was same-file only,
//! causing cross-file calls to fall to `__external__` placeholders.
//! Fix: mirror REFERENCES' repo-wide fallback at indexer.rs:225-233.
//!
//! See `docs/investigate/cross-file-calls-resolution-2026-04-22.md`.

use ga_index::Store;
use ga_query::indexer::build_index;
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

#[test]
fn cross_file_python_call_resolves_to_real_symbol() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // a.py defines alpha; b.py imports + calls it.
    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef b_caller():\n    alpha()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let conn = store.connection().unwrap();
    // The CALLS edge must point at the REAL `alpha` symbol in a.py,
    // not the `__external__::alpha` placeholder.
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'b_caller'})-[:CALLS]->(callee:Symbol {name: 'alpha'}) \
             RETURN callee.file, callee.kind",
        )
        .unwrap();
    let rows: Vec<(String, String)> = rs
        .into_iter()
        .filter_map(|r| {
            let cols: Vec<lbug::Value> = r.into_iter().collect();
            match (cols.first(), cols.get(1)) {
                (Some(lbug::Value::String(f)), Some(lbug::Value::String(k))) => {
                    Some((f.clone(), k.clone()))
                }
                _ => None,
            }
        })
        .collect();

    assert_eq!(
        rows.len(),
        1,
        "expected exactly one resolved edge: {rows:?}"
    );
    assert_eq!(rows[0].0, "a.py", "callee.file must be a.py, got {rows:?}");
    assert_ne!(
        rows[0].1, "external",
        "callee.kind must NOT be external (was resolved), got {rows:?}"
    );
}

#[test]
fn cross_file_call_does_not_create_external_placeholder_when_resolved() {
    // If alpha is defined repo-wide and the call resolves, there should be
    // NO `__external__::alpha` node leftover (the unique-def fallback finds
    // the real one directly).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef b_caller():\n    alpha()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {kind: 'external', name: 'alpha'}) RETURN count(s)")
        .unwrap();
    let n = rs
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next())
        .unwrap_or(lbug::Value::Int64(0));
    assert!(
        matches!(n, lbug::Value::Int64(0)),
        "no external alpha placeholder when a real alpha was resolved, got {n:?}"
    );
}

#[test]
fn unknown_cross_file_callee_still_becomes_external() {
    // Guard: when the callee name is NOT defined anywhere in the repo,
    // indexer must still synthesize the external placeholder
    // (pre-existing behavior, regression guard).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("b.py"),
        "def b_caller():\n    some_stdlib_fn()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {kind: 'external', name: 'some_stdlib_fn'}) RETURN count(s)")
        .unwrap();
    let n = rs
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next())
        .unwrap_or(lbug::Value::Int64(0));
    assert!(
        matches!(n, lbug::Value::Int64(1)),
        "unknown callee MUST produce external placeholder, got {n:?}"
    );
}
