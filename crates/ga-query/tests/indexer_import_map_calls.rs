//! Cross-file CALLS resolution — import-aware disambiguation (Phase B).
//!
//! When the callee name exists in >1 files, Phase A's `symbol_by_name`
//! fallback picks first-seen (alphabetical via walker). Phase B uses the
//! caller's `import` statement to pick the RIGHT file.
//!
//! Covers Python `from X import Y` and TS/JS `import { Y } from './X'`.
//! Go + Rust deferred (Go: package-aware; Rust: Phase C scope).
//!
//! See docs/investigate/cross-file-calls-resolution-2026-04-22.md Phase B.

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

fn resolved_callee_file(store: &Store, caller_name: &str, callee_name: &str) -> Option<String> {
    let conn = store.connection().unwrap();
    let cypher = format!(
        "MATCH (c:Symbol {{name: '{caller_name}'}})-[:CALLS]->(callee:Symbol {{name: '{callee_name}'}}) \
         RETURN callee.file"
    );
    let rs = conn.query(&cypher).unwrap();
    rs.into_iter()
        .next()
        .and_then(|r| match r.into_iter().next() {
            Some(lbug::Value::String(f)) => Some(f),
            _ => None,
        })
}

#[test]
fn python_import_disambiguates_between_two_same_name_defs() {
    // a.py and c.py both define `alpha`. b.py imports FROM c explicitly.
    // Phase A would pick whichever symbol_by_name saw first (alphabetical).
    // Phase B must honor the `from c import alpha` and pick c.py.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def alpha():\n    pass\n"); // alphabetical first → Phase A picks this
    write(&repo.join("c.py"), "def alpha():\n    pass\n"); // truth
    write(
        &repo.join("b.py"),
        "from c import alpha\n\ndef b_caller():\n    alpha()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let callee_file = resolved_callee_file(&store, "b_caller", "alpha");
    assert_eq!(
        callee_file,
        Some("c.py".to_string()),
        "import from c MUST resolve to c.py, not a.py (Phase A first-match would pick a.py)"
    );
}

#[test]
fn typescript_import_disambiguates_between_two_same_name_defs() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Two modules export `alpha`; b.ts imports from ./c specifically.
    write(&repo.join("a.ts"), "export function alpha() {}\n");
    write(&repo.join("c.ts"), "export function alpha() {}\n");
    write(
        &repo.join("b.ts"),
        "import { alpha } from './c';\nexport function b_caller() { alpha(); }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let callee_file = resolved_callee_file(&store, "b_caller", "alpha");
    assert_eq!(
        callee_file,
        Some("c.ts".to_string()),
        "TS named import from ./c MUST resolve to c.ts"
    );
}

// Aliased imports (`from c import alpha as beta`) need parser-side support
// to record `local_name → original_name` pairs — the current
// `imported_names` extractor returns only local names. Aliased resolution
// is tracked as a Phase B.1 follow-up; this test documents the gap and
// is #[ignore]d rather than deleted so it lights up if alias support lands.
#[test]
#[ignore = "Phase B.1 follow-up — parser must emit (local, original) pairs"]
fn python_aliased_import_resolves_original_name() {
    // `from c import alpha as beta` — calling `beta()` should resolve to
    // `c.py::alpha` (the original def; alias is just a local rename).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("c.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from c import alpha as beta\n\ndef b_caller():\n    beta()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // The caller code calls `beta` (the local alias). The graph should show
    // an edge to the ORIGINAL name `alpha` in c.py — that's what the alias
    // points at. If import_map is wired correctly, the resolver checks
    // (b.py, "beta") → c.py and finds c.py::alpha by its real name.
    //
    // Implementation note: parser's python_imported_names collapses aliases
    // to the aliased local name (what the caller typed). The callee_name
    // in pending_calls will be "beta" (what the call wrote) — resolving
    // (b.py, "beta") → c.py, then looking up "beta" in c.py — MISS.
    // So aliased imports hit a known limitation of the simple import_map
    // model unless we also record `local_name → original_name` mapping.
    //
    // This test asserts the CURRENT behavior after Phase B: when alias
    // resolution hits the miss, fallback to repo-wide finds `alpha` — still
    // gets the right file. Acceptable Phase B behavior; full alias support
    // can land in Phase B.1.
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'b_caller'})-[:CALLS]->(callee:Symbol) \
             RETURN callee.name, callee.file, callee.kind",
        )
        .unwrap();
    let rows: Vec<(String, String, String)> = rs
        .into_iter()
        .filter_map(|r| {
            let cols: Vec<lbug::Value> = r.into_iter().collect();
            match (cols.first(), cols.get(1), cols.get(2)) {
                (
                    Some(lbug::Value::String(n)),
                    Some(lbug::Value::String(f)),
                    Some(lbug::Value::String(k)),
                ) => Some((n.clone(), f.clone(), k.clone())),
                _ => None,
            }
        })
        .collect();
    // At least one edge exists pointing at a non-external alpha in c.py.
    assert!(
        rows.iter()
            .any(|(n, f, k)| n == "alpha" && f == "c.py" && k != "external"),
        "aliased import must eventually resolve to c.py::alpha (via repo-wide fallback): {rows:?}"
    );
}
