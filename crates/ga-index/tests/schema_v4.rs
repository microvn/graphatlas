//! v1.3 schema v4 — DDL scaffolding tests (PR1 scope).
//!
//! Covers spec lines:
//! - Schema v4 — Symbol +13 cols, File +5 cols
//! - Schema v4 — IMPLEMENTS / CALLS_HEURISTIC / IMPORTS_NAMED / DECORATES REL tables
//! - SCHEMA_VERSION constant 3 → 4
//! - Tools-C9.T3/T4 — `Store::open` uses BASE_DDL only; reopen of v4 cache does
//!   NOT re-emit ALTERs (no DDL error on second open).
//!
//! Out of scope (PR2): MIGRATION_STATEMENTS execution, run_schema_migration,
//! ALTER rollback, FS snapshot fallback.

use ga_index::{Store, SCHEMA_VERSION};
use std::path::Path;
use tempfile::TempDir;

fn fresh_store(tmp: &TempDir, repo: &str) -> Store {
    let cache_root = tmp.path().join(".graphatlas");
    Store::open_with_root(&cache_root, Path::new(repo)).unwrap()
}

#[test]
fn schema_version_pin() {
    // Versioned pin — updated on each schema bump. v1.3 shipped 4; v1.4
    // bumps 4→5 (Tools-C20). Future bumps update this assertion AND add
    // their own per-version pin file (`schema_v5.rs`, `schema_v6.rs`, ...).
    // The pin lives in the v1.3 file because v1.3's other column-shape
    // tests below exercise the same DDL — moving them would scatter the
    // schema-evolution audit trail.
    assert_eq!(SCHEMA_VERSION, 5, "v1.4 spec bumps SCHEMA_VERSION 4→5");
}

#[test]
fn v4_symbol_node_has_new_columns() {
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/v4-symbol-cols");
    let conn = store.connection().unwrap();
    // Insert a Symbol with v3 fields only — DEFAULTs cover v4 cols.
    conn.query(
        "CREATE (:Symbol {id: 's1', name: 'foo', file: 'a.py', \
         kind: 'function', line: 1, line_end: 5})",
    )
    .unwrap_or_else(|e| panic!("insert v3-shape Symbol: {e}"));
    // Each v4 column must be queryable; defaults must apply.
    for (col, expected_default) in [
        ("qualified_name", "''"),
        ("return_type", "''"),
        ("doc_summary", "''"),
    ] {
        let q = format!("MATCH (s:Symbol {{id: 's1'}}) RETURN s.{col} = {expected_default}");
        let rs = conn
            .query(&q)
            .unwrap_or_else(|e| panic!("v4 col {col} missing: {e}"));
        let mut got_true = false;
        for row in rs {
            if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
                got_true = b;
            }
        }
        assert!(got_true, "v4 col {col} default mismatch");
    }
    // Boolean defaults
    for col in [
        "is_async",
        "is_override",
        "is_abstract",
        "is_static",
        "is_test_marker",
        "is_generated",
    ] {
        let q = format!("MATCH (s:Symbol {{id: 's1'}}) RETURN s.{col}");
        let rs = conn
            .query(&q)
            .unwrap_or_else(|e| panic!("v4 bool col {col} missing: {e}"));
        for row in rs {
            if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
                assert!(!b, "v4 bool col {col} default should be false");
            }
        }
    }
    // arity default = -1 (unknown sentinel per Tools-C2)
    let rs = conn
        .query("MATCH (s:Symbol {id: 's1'}) RETURN s.arity")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, -1, "arity default per Tools-C2");
        }
    }
    // confidence default = 1.0 (DOUBLE not FLOAT — kuzu#5159 workaround,
    // applies to both CREATE-with-DEFAULT (PR1) and ALTER ADD (PR2) paths)
    let rs = conn
        .query("MATCH (s:Symbol {id: 's1'}) RETURN s.confidence")
        .unwrap();
    for row in rs {
        match row.into_iter().next() {
            Some(lbug::Value::Float(f)) => assert!((f - 1.0).abs() < 1e-6),
            Some(lbug::Value::Double(f)) => assert!((f - 1.0).abs() < 1e-6),
            other => panic!("confidence wrong type: {other:?}"),
        }
    }
}

#[test]
fn pr2_migration_statements_empty_in_v1_3() {
    // PR2 finding: ALTER pattern triggers kuzu#6045 across the test harness.
    // Net user-value over PR1 CREATE-with-DEFAULT is 0 (same final schema,
    // same lifecycle). MIGRATION_STATEMENTS stays empty in v1.3, reserved
    // for v5+ when kuzu#6045 family fixes land upstream.
    let migration = ga_index::schema::MIGRATION_STATEMENTS;
    assert_eq!(
        migration.len(),
        0,
        "v1.3 ships MIGRATION_STATEMENTS empty per PR2 finding; got {} statements",
        migration.len()
    );
}

#[test]
fn pr2_base_ddl_holds_full_v4_schema() {
    // PR1 pattern preserved: v4 cols inline in CREATE-with-DEFAULT. confidence
    // DOUBLE per Tools-C12. Composites (params, modifiers) deferred per
    // Tools-C13.
    let base = ga_index::schema::BASE_DDL_STATEMENTS;
    let joined = base.join("\n");
    let symbol_block = joined
        .split("CREATE NODE TABLE IF NOT EXISTS Symbol")
        .nth(1)
        .unwrap_or("")
        .split("CREATE")
        .next()
        .unwrap_or("");
    // v4 scalar/boolean/string cols inline in CREATE
    for col in [
        "qualified_name STRING DEFAULT",
        "return_type STRING DEFAULT",
        "arity INT64 DEFAULT",
        "confidence DOUBLE DEFAULT",
        "is_async BOOLEAN DEFAULT",
        "doc_summary STRING DEFAULT",
    ] {
        assert!(
            symbol_block.contains(col),
            "v4 col `{col}` must be in BASE Symbol CREATE"
        );
    }
    // PR5c1 supersedes Tools-C13: composites ship via CREATE-with-DEFAULT
    // pattern after spike_pr5c_store.rs proved kuzu#6045 was ALTER-specific.
    assert!(
        symbol_block.contains("params STRUCT"),
        "params STRUCT(...)[] must be in BASE Symbol CREATE (PR5c1)"
    );
    assert!(
        symbol_block.contains("modifiers STRING["),
        "modifiers STRING[] must be in BASE Symbol CREATE (PR5c1)"
    );
    // v4 REL tables present
    for rel in [
        "IMPLEMENTS",
        "CALLS_HEURISTIC",
        "IMPORTS_NAMED",
        "DECORATES",
    ] {
        assert!(
            joined.contains(&format!("CREATE REL TABLE IF NOT EXISTS {rel}")),
            "v4 REL {rel} must be in BASE_DDL_STATEMENTS"
        );
    }
}

#[test]
fn v4_file_node_has_new_columns() {
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/v4-file-cols");
    let conn = store.connection().unwrap();
    // Insert with v3 shape only; v4 cols use DEFAULTs.
    conn.query("CREATE (:File {path: 'a.py', lang: 'python', size: 100})")
        .unwrap_or_else(|e| panic!("insert v3-shape File: {e}"));
    // loc default = 0
    let rs = conn
        .query("MATCH (f:File {path: 'a.py'}) RETURN f.loc")
        .unwrap_or_else(|e| panic!("loc col missing: {e}"));
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, 0, "loc default 0");
        }
    }
    // is_generated / is_vendored default false
    for col in ["is_generated", "is_vendored"] {
        let q = format!("MATCH (f:File {{path: 'a.py'}}) RETURN f.{col}");
        let rs = conn
            .query(&q)
            .unwrap_or_else(|e| panic!("v4 col {col} missing: {e}"));
        for row in rs {
            if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
                assert!(!b, "v4 col {col} default false");
            }
        }
    }
    // sha256 / modified_at exist as columns (NULL by default — query must
    // succeed without column-not-found error)
    conn.query("MATCH (f:File {path: 'a.py'}) RETURN f.sha256, f.modified_at")
        .unwrap_or_else(|e| panic!("sha256/modified_at columns missing: {e}"));
}

#[test]
fn v4_new_rel_tables_exist() {
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/v4-rels");
    let conn = store.connection().unwrap();
    for rel in [
        "IMPLEMENTS",
        "CALLS_HEURISTIC",
        "IMPORTS_NAMED",
        "DECORATES",
    ] {
        let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
        conn.query(&q)
            .unwrap_or_else(|e| panic!("v4 rel table {rel} missing: {e}"));
    }
}

#[test]
fn v3_rel_tables_still_exist() {
    // v1.3-Tools-C7 strict-union catch-all: v3 RELs preserved.
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/v3-preserved");
    let conn = store.connection().unwrap();
    for rel in [
        "CALLS",
        "IMPORTS",
        "DEFINES",
        "EXTENDS",
        "TESTED_BY",
        "REFERENCES",
        "MODULE_TYPED",
        "CONTAINS",
    ] {
        let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
        conn.query(&q)
            .unwrap_or_else(|e| panic!("v3 rel table {rel} missing: {e}"));
    }
}

#[test]
fn store_reopen_does_not_replay_alters() {
    // v1.3-Tools-C9: BASE_DDL_STATEMENTS is idempotent (CREATE … IF NOT
    // EXISTS); MIGRATION_STATEMENTS gated on `!Resumed` so future ALTER
    // additions don't replay-and-error on already-migrated cache.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    // v1.5 PR2 AS-001: real path required for commit (Merkle root hash).
    let repo_dir = tmp.path().join("repos").join("v4-reopen");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();
    let s1 = Store::open_with_root(&cache_root, repo).unwrap();
    s1.commit().unwrap();
    let s2 = Store::open_with_root(&cache_root, repo)
        .expect("reopen of v4 cache must not error — Tools-C9 BASE_DDL idempotent");
    let conn = s2.connection().unwrap();
    // Verify v4 cols persisted across reopen (PR1 CREATE-with-DEFAULT pattern).
    conn.query("MATCH (s:Symbol) RETURN count(s.qualified_name)")
        .expect("v4 qualified_name col must persist across reopen");
    conn.query("MATCH (f:File) RETURN count(f.sha256)")
        .expect("v4 sha256 col must persist across reopen");
}
