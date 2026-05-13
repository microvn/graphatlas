//! v1.3 PR10 — `ga_hubs` excludes CALLS_HEURISTIC by default; opt-in via
//! `edge_types: "all"` (S-007 AS-018 closure).
//!
//! Spec: spec, AS-018.
//!
//! Then-clause: default hub degree = `count(catch-all CALLS) − count(CALLS_HEURISTIC)`
//! plus REFERENCES/EXTENDS/TESTED_BY/CONTAINS unchanged. `edge_types: "all"`
//! drops the subtraction → counts every CALLS row including heuristic.

use ga_index::Store;
use ga_query::hubs::{hubs, HubsEdgeTypes, HubsRequest};
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

fn helper_in_degree(store: &Store, edge_types: HubsEdgeTypes) -> u32 {
    let req = HubsRequest {
        top_n: 100,
        symbol: Some("helper".to_string()),
        file: None,
        edge_types,
    };
    let resp = hubs(store, &req).unwrap();
    resp.hubs
        .iter()
        .find(|h| h.name == "helper")
        .map(|h| h.in_degree)
        .unwrap_or(0)
}

#[test]
fn default_excludes_calls_heuristic_from_in_degree() {
    // a.py defines helper. b.py + c.py call helper() WITHOUT importing it
    // (tier-3 repo-wide fallback → both edges land in CALLS_HEURISTIC).
    // Default hub degree should be 0 (heuristic edges excluded).
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(repo.path(), "b.py", "def b1():\n    return helper()\n");
    write_file(repo.path(), "c.py", "def c1():\n    return helper()\n");
    let (_t, store) = index_repo(repo.path());
    let n = helper_in_degree(&store, HubsEdgeTypes::Default);
    assert_eq!(
        n, 0,
        "default mode must exclude CALLS_HEURISTIC; helper in-degree should be 0, got {n}"
    );
}

#[test]
fn all_mode_includes_calls_heuristic() {
    // Same fixture; "all" mode counts every CALLS row including heuristic.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(repo.path(), "b.py", "def b1():\n    return helper()\n");
    write_file(repo.path(), "c.py", "def c1():\n    return helper()\n");
    let (_t, store) = index_repo(repo.path());
    let n = helper_in_degree(&store, HubsEdgeTypes::All);
    assert_eq!(
        n, 2,
        "`edge_types: all` must count heuristic edges; helper in-degree = 2, got {n}"
    );
}

#[test]
fn confident_calls_count_in_default_mode() {
    // Same-file callers (tier-1 — confident) PLUS cross-file callers
    // (tier-3 — heuristic). Default should count only the confident edge.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "a.py",
        "def helper():\n    return 1\n\ndef same_file_caller():\n    return helper()\n",
    );
    write_file(
        repo.path(),
        "b.py",
        "def cross_file_caller():\n    return helper()\n",
    );
    let (_t, store) = index_repo(repo.path());
    let default_n = helper_in_degree(&store, HubsEdgeTypes::Default);
    let all_n = helper_in_degree(&store, HubsEdgeTypes::All);
    assert_eq!(default_n, 1, "default = 1 confident edge, got {default_n}");
    assert_eq!(all_n, 2, "all = 1 confident + 1 heuristic = 2, got {all_n}");
}

#[test]
fn default_mode_request_is_default_value() {
    // `HubsRequest::default()` should pick HubsEdgeTypes::Default — the
    // safer mode per AS-018. Backward-compat for existing callers.
    let req = HubsRequest::default();
    assert!(matches!(req.edge_types, HubsEdgeTypes::Default));
}
