//! v1.3 PR12 — Consolidated v4 schema audit suite (CI gate).
//!
//! Single fixture, single index pass, all 6 v4 audits in one test file. AT-001
//! through AT-008 per spec — only 6 are defined (AT-004, AT-006 don't appear
//! anywhere in spec text). AT-007 thread-count requirement (Tools-C5) is N/A
//! v1.3 — walker is single-threaded; only the re-index portion is asserted.
//!
//! Spec: spec
//! Defined audits:
//! - AT-001 — qualified_name uniqueness (no empty for non-external rows)
//! - AT-002 — `arity == size(params)` for symbols with non-empty params
//! - AT-003 — every IMPORTS_NAMED row has matching legacy IMPORTS at same line
//! - AT-005 — zero File rows have NULL sha256
//! - AT-007 — qualified_name byte-identical across re-index of same content
//! - AT-008 — variant subset cardinality: count(CALLS) ≥ count(CALLS_HEURISTIC),
//!            count(EXTENDS) ≥ count(IMPLEMENTS); each variant row has matching
//!            catch-all row
//!
//! These audits also live inline in PR-of-origin tests
//! (pr4_qualified_name.rs, pr5_arity.rs, pr6_file_metadata.rs, pr7_imports_named.rs,
//! pr5c2a_signature.rs, pr9_strict_union.rs) — this file is the consolidated
//! CI gate that runs them as a single sweep against a richer multi-lang fixture.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

/// Multi-language fixture exercising every v4 column + variant table.
fn build_fixture(repo: &Path) {
    // Python — decorator + import named + class hierarchy
    write_file(repo, "pkg/__init__.py", "");
    write_file(
        repo,
        "pkg/decorators.py",
        "def my_dec(fn):\n    return fn\n",
    );
    write_file(
        repo,
        "pkg/main.py",
        "from pkg.decorators import my_dec\n\
         \n\
         @my_dec\n\
         def target_fn(x: int, y: int = 10) -> int:\n    return x + y\n\
         \n\
         class Base:\n    pass\n\
         \n\
         class Child(Base):\n    def m(self):\n        return target_fn(1, 2)\n",
    );
    // Rust — struct + impl + trait + heuristic-resolvable cross-file call
    write_file(
        repo,
        "src/lib.rs",
        "pub trait Greet { fn hello(&self) -> String; }\n\
         pub struct Foo;\n\
         impl Greet for Foo { fn hello(&self) -> String { String::from(\"hi\") } }\n\
         \n\
         pub fn helper() -> i32 { 1 }\n",
    );
    write_file(repo, "src/use.rs", "fn caller() -> i32 { helper() }\n");
    // Java — class implements interface
    write_file(repo, "I.java", "interface I { void run(); }\n");
    write_file(
        repo,
        "C.java",
        "class C implements I { public void run() {} }\n",
    );
}

fn open_and_index(repo: &Path, cache_root: &Path) -> Store {
    // Use a subdir so its perms aren't tempfile's 0755 default.
    let cache = cache_root.join(".graphatlas");
    let store = Store::open_with_root(&cache, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    Store::open_with_root(&cache, repo).unwrap()
}

fn count(store: &Store, q: &str) -> i64 {
    let conn = store.connection().unwrap();
    let rs = conn.query(q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return n;
        }
    }
    0
}

#[test]
fn at_001_qualified_name_no_empty_for_non_external() {
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let n = count(
        &store,
        "MATCH (s:Symbol) WHERE s.qualified_name = '' AND s.kind <> 'external' RETURN count(s)",
    );
    assert_eq!(
        n, 0,
        "AT-001: zero non-external rows may have empty qualified_name"
    );
}

#[test]
fn at_002_arity_equals_size_params_when_populated() {
    // Tools-C2: arity == size(params) when params != [].
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let conn = store.connection().unwrap();
    // Find rows with populated params and verify cardinality equality.
    let rs = conn
        .query(
            "MATCH (s:Symbol) WHERE size(s.params) > 0 \
             RETURN s.name, s.arity, size(s.params)",
        )
        .unwrap();
    let mut checked = 0;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        let arity = match cols.get(1) {
            Some(lbug::Value::Int64(n)) => *n,
            _ => continue,
        };
        let psize = match cols.get(2) {
            Some(lbug::Value::Int64(n)) => *n,
            _ => continue,
        };
        assert_eq!(
            arity, psize,
            "AT-002: arity ({arity}) must equal size(params) ({psize})"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "AT-002: expected ≥1 symbol with populated params"
    );
}

#[test]
fn at_003_imports_named_overlaps_legacy_imports() {
    // Every IMPORTS_NAMED row's import_line should match a legacy IMPORTS
    // row at same (src_file, import_line).
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File)-[r:IMPORTS_NAMED]->(:Symbol) RETURN f.path, r.import_line")
        .unwrap();
    let mut named_pairs: Vec<(String, i64)> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if let (Some(lbug::Value::String(p)), Some(lbug::Value::Int64(l))) =
            (cols.first(), cols.get(1))
        {
            named_pairs.push((p.clone(), *l));
        }
    }
    assert!(
        !named_pairs.is_empty(),
        "AT-003: expected ≥1 IMPORTS_NAMED row"
    );
    for (path, line) in &named_pairs {
        let q = format!(
            "MATCH (f:File {{path: '{path}'}})-[r:IMPORTS]->(:File) \
             WHERE r.import_line = {line} RETURN count(r)"
        );
        let n = count(&store, &q);
        assert!(
            n >= 1,
            "AT-003: IMPORTS_NAMED ({path}, line {line}) must have matching legacy IMPORTS row"
        );
    }
}

#[test]
fn at_005_no_null_sha256() {
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let n = count(
        &store,
        "MATCH (f:File) WHERE f.sha256 IS NULL RETURN count(f)",
    );
    assert_eq!(n, 0, "AT-005: zero File rows may have NULL sha256");
}

#[test]
fn at_007_qualified_name_stable_across_reindex() {
    // Tools-C5 / AT-007 — re-index of identical content yields identical
    // qualified_names. Thread-count portion ({1,4,16}) N/A v1.3 — walker
    // single-threaded.
    let tmp_a = TempDir::new().unwrap();
    build_fixture(tmp_a.path());
    let cache_a = TempDir::new().unwrap();
    let store_a = open_and_index(tmp_a.path(), cache_a.path());

    let tmp_b = TempDir::new().unwrap();
    build_fixture(tmp_b.path());
    let cache_b = TempDir::new().unwrap();
    let store_b = open_and_index(tmp_b.path(), cache_b.path());

    let collect = |s: &Store| -> Vec<String> {
        let conn = s.connection().unwrap();
        let rs = conn
            .query(
                "MATCH (s:Symbol) WHERE s.kind <> 'external' \
                 RETURN s.qualified_name ORDER BY s.qualified_name",
            )
            .unwrap();
        let mut out = Vec::new();
        for row in rs {
            if let Some(lbug::Value::String(s)) = row.into_iter().next() {
                out.push(s);
            }
        }
        out
    };
    let a = collect(&store_a);
    let b = collect(&store_b);
    assert_eq!(
        a, b,
        "AT-007: qualified_name must be byte-identical across re-index"
    );
    assert!(!a.is_empty(), "AT-007: expected non-empty Symbol set");
}

#[test]
fn at_008_strict_union_cardinality() {
    // Tools-C7 strict-union: catch-all is superset of every variant.
    // count(CALLS) ≥ count(CALLS_HEURISTIC); count(EXTENDS) ≥ count(IMPLEMENTS).
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let calls = count(&store, "MATCH ()-[r:CALLS]->() RETURN count(r)");
    let calls_h = count(&store, "MATCH ()-[r:CALLS_HEURISTIC]->() RETURN count(r)");
    assert!(
        calls >= calls_h,
        "AT-008: CALLS ({calls}) must be ≥ CALLS_HEURISTIC ({calls_h})"
    );
    let extends = count(&store, "MATCH ()-[r:EXTENDS]->() RETURN count(r)");
    let implements = count(&store, "MATCH ()-[r:IMPLEMENTS]->() RETURN count(r)");
    assert!(
        extends >= implements,
        "AT-008: EXTENDS ({extends}) must be ≥ IMPLEMENTS ({implements})"
    );
    assert!(
        implements >= 1,
        "fixture should produce ≥1 IMPLEMENTS row (Java C→I)"
    );
}

#[test]
fn at_008_implements_subset_matches_extends_pairs() {
    // Strict-union content: every IMPLEMENTS (src, dst) pair has matching
    // EXTENDS row.
    let tmp = TempDir::new().unwrap();
    build_fixture(tmp.path());
    let cache = TempDir::new().unwrap();
    let store = open_and_index(tmp.path(), cache.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s)-[:IMPLEMENTS]->(t) RETURN s.id, t.id")
        .unwrap();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if let (Some(lbug::Value::String(s)), Some(lbug::Value::String(t))) =
            (cols.first(), cols.get(1))
        {
            pairs.push((s.clone(), t.clone()));
        }
    }
    for (s, t) in &pairs {
        let q = format!(
            "MATCH (a:Symbol {{id: '{s}'}})-[r:EXTENDS]->(b:Symbol {{id: '{t}'}}) RETURN count(r)"
        );
        let n = count(&store, &q);
        assert!(
            n >= 1,
            "AT-008 strict-union: IMPLEMENTS ({s} → {t}) must have matching EXTENDS row"
        );
    }
}
