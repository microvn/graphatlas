//! v1.3 PR9 — Strict-union catch-all (S-007 AS-017 partial).
//!
//! Spec: spec, S-007.
//!
//! Scope (PR9a + PR9b combined this session):
//! - CALLS_HEURISTIC: tier-3 repo-wide-fallback CALLS edges ALSO write a row
//!   into CALLS_HEURISTIC. Catch-all CALLS preserved (every edge writes
//!   there too). Tools-C7 invariant: count(CALLS) ≥ count(CALLS_HEURISTIC).
//! - IMPLEMENTS: when EXTENDS target's `kind` is `interface` or `trait`,
//!   ALSO emit an IMPLEMENTS row. Catch-all EXTENDS preserved.
//!
//! AT-008 audit: every variant row has matching catch-all row at same
//! `(caller, callee, line)` (CALLS) or `(child, parent)` (EXTENDS).

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

fn count_rel(store: &Store, rel: &str) -> i64 {
    let conn = store.connection().unwrap();
    let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return n;
        }
    }
    0
}

// ─────────────────────────────────────────────────────────────────────────
// CALLS_HEURISTIC — tier-3 repo-wide fallback
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn cross_file_repo_wide_fallback_emits_calls_heuristic() {
    // a.py defines `helper`. b.py calls `helper()` WITHOUT importing it
    // (heuristic resolution via repo-wide single-def lookup, tier 3 of
    // the indexer's resolution priority). Expect:
    // - 1 CALLS row (catch-all preserved)
    // - 1 CALLS_HEURISTIC row (tier-3 emit)
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(repo.path(), "b.py", "def caller():\n    return helper()\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    // CALLS catch-all has the edge (existing behavior)
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'caller'})-[:CALLS]->(t:Symbol {name: 'helper'}) RETURN count(*)",
        )
        .unwrap();
    let mut calls_n = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            calls_n = n;
        }
    }
    assert_eq!(calls_n, 1, "CALLS catch-all must include heuristic edge");
    // CALLS_HEURISTIC also has it (PR9a new)
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'caller'})-[:CALLS_HEURISTIC]->(t:Symbol {name: 'helper'}) RETURN count(*)",
        )
        .unwrap();
    let mut heur_n = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            heur_n = n;
        }
    }
    assert_eq!(
        heur_n, 1,
        "PR9a: tier-3 repo-wide fallback must emit CALLS_HEURISTIC"
    );
}

#[test]
fn same_file_call_does_not_emit_calls_heuristic() {
    // Tier-1 resolution (same-file) → confident edge; CALLS_HEURISTIC must
    // NOT have a row. Catch-all CALLS still gets it.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def helper():\n    return 1\n\ndef caller():\n    return helper()\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'caller'})-[:CALLS_HEURISTIC]->(t:Symbol {name: 'helper'}) RETURN count(*)",
        )
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, 0, "same-file resolution must NOT be heuristic, got {n}");
        }
    }
}

#[test]
fn calls_heuristic_is_subset_of_calls_catchall() {
    // AT-008 cardinality: count(CALLS) ≥ count(CALLS_HEURISTIC) — strict
    // superset invariant per Tools-C7.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(
        repo.path(),
        "b.py",
        "def b1():\n    return helper()\n\ndef b2():\n    return helper()\n",
    );
    let (_t, store) = index_repo(repo.path());
    let calls = count_rel(&store, "CALLS");
    let heur = count_rel(&store, "CALLS_HEURISTIC");
    assert!(
        calls >= heur,
        "Tools-C7 strict superset: CALLS ({calls}) >= CALLS_HEURISTIC ({heur})"
    );
    assert!(heur >= 2, "expected ≥2 heuristic edges, got {heur}");
}

// ─────────────────────────────────────────────────────────────────────────
// IMPLEMENTS — interface/trait targets ALSO write IMPLEMENTS
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn java_class_implements_interface_emits_both_edges() {
    // Java `class C implements I` → both EXTENDS (catch-all) and IMPLEMENTS.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "I.java", "interface I { void run(); }\n");
    write_file(
        repo.path(),
        "C.java",
        "class C implements I { public void run() {} }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    // EXTENDS catch-all has the edge
    let rs = conn
        .query("MATCH (c:Symbol {name: 'C'})-[:EXTENDS]->(i:Symbol {name: 'I'}) RETURN count(*)")
        .unwrap();
    let mut e_n = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            e_n = n;
        }
    }
    assert_eq!(e_n, 1, "EXTENDS catch-all must include C→I");
    // IMPLEMENTS has it too
    let rs = conn
        .query("MATCH (c:Symbol {name: 'C'})-[:IMPLEMENTS]->(i:Symbol {name: 'I'}) RETURN count(*)")
        .unwrap();
    let mut i_n = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            i_n = n;
        }
    }
    assert_eq!(i_n, 1, "PR9b: Java class→interface must emit IMPLEMENTS");
}

#[test]
fn java_class_extends_class_only_no_implements() {
    // `class B extends A` (A is class, not interface) → EXTENDS only,
    // no IMPLEMENTS.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "A.java", "class A {}\n");
    write_file(repo.path(), "B.java", "class B extends A {}\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (b:Symbol {name: 'B'})-[:IMPLEMENTS]->(a:Symbol {name: 'A'}) RETURN count(*)")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(
                n, 0,
                "class-extends-class must NOT emit IMPLEMENTS, got {n}"
            );
        }
    }
}

#[test]
fn rust_impl_trait_for_type_emits_implements() {
    // Rust `impl Display for Foo` — Foo "extends" trait Display →
    // EXTENDS catch-all + IMPLEMENTS (trait target).
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "trait Display { fn fmt(&self); }\nstruct Foo;\nimpl Display for Foo { fn fmt(&self) {} }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (f:Symbol {name: 'Foo'})-[:IMPLEMENTS]->(d:Symbol {name: 'Display'}) RETURN count(*)",
        )
        .unwrap();
    let mut n_implements = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            n_implements = n;
        }
    }
    assert_eq!(
        n_implements, 1,
        "Rust impl Trait for Type must emit IMPLEMENTS, got {n_implements}"
    );
}

#[test]
fn implements_is_subset_of_extends_catchall() {
    // AT-008: count(EXTENDS) ≥ count(IMPLEMENTS) — strict superset.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.java",
        "interface I {} class A {} class B extends A implements I {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let extends = count_rel(&store, "EXTENDS");
    let implements = count_rel(&store, "IMPLEMENTS");
    assert!(
        extends >= implements,
        "Tools-C7 strict superset: EXTENDS ({extends}) >= IMPLEMENTS ({implements})"
    );
    assert!(
        implements >= 1,
        "expected ≥1 IMPLEMENTS row, got {implements}"
    );
}
