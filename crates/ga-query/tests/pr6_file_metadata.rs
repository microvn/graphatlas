//! v1.3 PR6 — File operational metadata (S-005 AS-013).
//!
//! Spec: spec, S-005.
//!
//! Then-clause requirements:
//! - sha256 != NULL (32-byte BLAKE3 in BLOB form)
//! - modified_at != NULL (TIMESTAMP)
//! - loc > 0
//! - is_generated populated (path patterns: "generated/", "*.gen.go", etc.)
//! - is_vendored populated (node_modules/, vendor/, third_party/)
//! - (path, sha256) byte-identical pre/post reindex (deterministic hash)

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

#[test]
fn sha256_populated_32_bytes_blob() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "main.py", "def hello():\n    return 1\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'main.py'}) RETURN f.sha256")
        .unwrap();
    let mut found = false;
    for row in rs {
        match row.into_iter().next() {
            Some(lbug::Value::Blob(b)) => {
                assert_eq!(b.len(), 32, "BLAKE3 hash must be 32 bytes, got {}", b.len());
                assert!(!b.iter().all(|x| *x == 0), "hash must not be all zeros");
                found = true;
            }
            other => panic!("expected Blob, got {other:?}"),
        }
    }
    assert!(found, "no File row");
}

#[test]
fn sha256_deterministic_across_reindex() {
    // Same content → same hash (foundation for v1.4 incremental rebuild).
    let repo_a = TempDir::new().unwrap();
    write_file(repo_a.path(), "a.py", "def x(): return 1\n");
    let (_t1, store_a) = index_repo(repo_a.path());
    let conn_a = store_a.connection().unwrap();
    let mut hash_a: Vec<u8> = Vec::new();
    let rs = conn_a
        .query("MATCH (f:File {path: 'a.py'}) RETURN f.sha256")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Blob(b)) = row.into_iter().next() {
            hash_a = b;
        }
    }

    let repo_b = TempDir::new().unwrap();
    write_file(repo_b.path(), "a.py", "def x(): return 1\n");
    let (_t2, store_b) = index_repo(repo_b.path());
    let conn_b = store_b.connection().unwrap();
    let mut hash_b: Vec<u8> = Vec::new();
    let rs = conn_b
        .query("MATCH (f:File {path: 'a.py'}) RETURN f.sha256")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Blob(b)) = row.into_iter().next() {
            hash_b = b;
        }
    }
    assert_eq!(
        hash_a, hash_b,
        "BLAKE3 must be deterministic for identical content"
    );
}

#[test]
fn loc_counts_lines() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "line1\nline2\nline3\nline4\nline5\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'main.py'}) RETURN f.loc")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            assert_eq!(n, 5, "loc must count newlines");
        }
    }
}

#[test]
fn modified_at_populated_not_null() {
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "x.py", "pass\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'x.py'}) RETURN f.modified_at IS NOT NULL")
        .unwrap();
    let mut got = false;
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            assert!(b, "modified_at must be populated");
            got = true;
        }
    }
    assert!(got, "no row");
}

#[test]
fn is_vendored_path_helper_recognizes_common_prefixes() {
    // Walker filters node_modules / vendor / etc. before indexing — so they
    // never become File rows. The is_vendored field exists for cases where
    // a vendored path slips through the walker filter (e.g., user override
    // via config). Test the heuristic via a `vendor/` subdir which the
    // walker may NOT exclude (Go vendor dirs are convention not hard-skip).
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "src/main.go", "package main\nfunc M() {}\n");
    write_file(
        repo.path(),
        "vendor/github.com/foo/bar.go",
        "package bar\nfunc B() {}\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'src/main.go'}) RETURN f.is_vendored")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            assert!(!b, "src/main.go must NOT be is_vendored");
        }
    }
    // vendor/* — if walker indexes it, must mark vendored. If walker filters
    // it, this test is a no-op (no row).
    let rs = conn
        .query("MATCH (f:File) WHERE f.path STARTS WITH 'vendor/' RETURN f.is_vendored LIMIT 1")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            assert!(
                b,
                "vendor/ paths that DO get indexed must have is_vendored = true"
            );
        }
    }
}

#[test]
fn is_generated_for_pb_go_path() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "proto/foo.pb.go",
        "package proto\n\nfunc Bar() {}\n",
    );
    write_file(repo.path(), "main.go", "package main\n\nfunc Main() {}\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File {path: 'proto/foo.pb.go'}) RETURN f.is_generated")
        .unwrap();
    let mut got = None;
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            got = Some(b);
        }
    }
    assert_eq!(got, Some(true), "*.pb.go is_generated must be true");
    // Plain .go → not generated
    let rs = conn
        .query("MATCH (f:File {path: 'main.go'}) RETURN f.is_generated")
        .unwrap();
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            assert!(!b, "main.go must NOT be is_generated");
        }
    }
}

#[test]
fn at_005_audit_zero_null_sha256() {
    // Then clause: zero File rows with NULL sha256 post-index.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def a(): pass\n");
    write_file(repo.path(), "b.go", "package b\nfunc B() {}\n");
    write_file(repo.path(), "c.ts", "export const c = 1;\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (f:File) WHERE f.sha256 IS NULL RETURN count(f)")
        .unwrap();
    let mut n = i64::MAX;
    for row in rs {
        if let Some(lbug::Value::Int64(v)) = row.into_iter().next() {
            n = v;
        }
    }
    assert_eq!(n, 0, "AT-005: zero File rows may have NULL sha256");
}
