//! M1 — graph schema DDL on Store::open. Idempotent CREATE NODE/REL tables
//! so downstream indexer + query layers can assume the tables exist.

use ga_index::Store;
use std::path::Path;
use tempfile::TempDir;

fn fresh_store(tmp: &TempDir, repo: &str) -> Store {
    let cache_root = tmp.path().join(".graphatlas");
    Store::open_with_root(&cache_root, Path::new(repo)).unwrap()
}

#[test]
fn schema_tables_exist_on_fresh_store_open() {
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/schema-smoke");
    let conn = store.connection().unwrap();
    // Each of these queries must run without error if the schema is in place.
    // We issue MATCH/count queries per table — zero rows is fine, but the
    // table must be known to lbug.
    for table in ["File", "Symbol"] {
        let q = format!("MATCH (n:{table}) RETURN count(n)");
        conn.query(&q)
            .unwrap_or_else(|e| panic!("table {table} missing: {e}"));
    }
    for rel in ["CALLS", "IMPORTS", "DEFINES", "EXTENDS", "TESTED_BY"] {
        let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
        conn.query(&q)
            .unwrap_or_else(|e| panic!("rel table {rel} missing: {e}"));
    }
}

#[test]
fn schema_is_idempotent_across_reopens() {
    // Open, commit, reopen → DDL must not error on second call.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    // v1.5 PR2 AS-001: real path required for commit (Merkle root hash).
    let repo_dir = tmp.path().join("repos").join("schema-reopen");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();

    let s1 = Store::open_with_root(&cache_root, repo).unwrap();
    s1.commit().unwrap();
    let s2 = Store::open_with_root(&cache_root, repo).unwrap();
    let conn = s2.connection().unwrap();
    conn.query("MATCH (n:File) RETURN count(n)").unwrap();
}

#[test]
fn can_insert_and_read_a_file_node() {
    // Smoke: schema actually accepts our primary-key shape.
    let tmp = TempDir::new().unwrap();
    let store = fresh_store(&tmp, "/work/schema-insert");
    let conn = store.connection().unwrap();
    conn.query("CREATE (:File {path: 'app.py', lang: 'python', size: 42})")
        .unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'app.py'}) RETURN f.size")
        .unwrap();
    let mut found = false;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, 42);
            found = true;
        }
    }
    assert!(found, "inserted File node not retrievable");
}
