//! `ga_bridges` — Brandes' betweenness centrality smoke tests.
//!
//! Goal: verify the implementation distinguishes bridge nodes from hub
//! nodes (different metric, different ranking on chosen topologies) and
//! that response shape + sampling meta are consistent.

use ga_index::Store;
use ga_query::bridges::{bridges, BridgesRequest};
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (tmp, cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn bridge_node_in_dumbbell_outranks_endpoints() {
    // Build a "dumbbell" topology:
    //   left_a, left_b, left_c → bridge   bridge → right_a, right_b, right_c
    // bridge sits on every shortest path between left-{a,b,c} and right-{a,b,c}
    // → must dominate the bridge ranking.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("d.py"),
        r#"
def bridge():
    return 0

def left_a(): return bridge()
def left_b(): return bridge()
def left_c(): return bridge()

def right_a(): return bridge()
def right_b(): return bridge()
def right_c(): return bridge()
"#,
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(
        &store,
        &BridgesRequest {
            top_n: 10,
            ..Default::default()
        },
    )
    .expect("bridges ok");
    assert!(!resp.bridges.is_empty(), "should have at least one bridge");
    let top = &resp.bridges[0];
    assert_eq!(top.name, "bridge", "bridge must rank #1; got {top:?}");
    assert!(top.betweenness > 0.0);
}

#[test]
fn empty_repo_returns_empty_bridges() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("e.py"), "# nothing\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(&store, &BridgesRequest::default()).expect("bridges ok");
    assert!(resp.bridges.is_empty());
    assert_eq!(resp.meta.total_nodes, 0);
    assert!(!resp.meta.sampled);
}

#[test]
fn top_n_caps_response() {
    let (_tmp, cache, repo) = setup();
    // Linear chain — every interior node has positive betweenness, so we
    // get N-2 candidates and can verify the top_n cap.
    let mut src = String::new();
    for i in 0..15 {
        src.push_str(&format!("def f{i}():\n    return f{}()\n\n", (i + 1) % 15));
    }
    write(&repo.join("c.py"), &src);

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(
        &store,
        &BridgesRequest {
            top_n: 3,
            ..Default::default()
        },
    )
    .expect("bridges ok");
    assert!(
        resp.bridges.len() <= 3,
        "top_n=3 must cap; got {}",
        resp.bridges.len()
    );
    for b in &resp.bridges {
        assert!(b.betweenness >= 0.0);
        assert!(!b.kind.is_empty(), "kind populated");
    }
}

// ─── S-004 — symbol-lookup mode ──────────────────────────────────────

#[test]
fn lookup_returns_betweenness_rank_for_known_symbol() {
    // AS-017 — bridges symbol-lookup mirror of hubs lookup.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("d.py"),
        r#"
def bridge():
    return 0

def left_a(): return bridge()
def left_b(): return bridge()

def right_a(): return bridge()
def right_b(): return bridge()
"#,
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(
        &store,
        &BridgesRequest {
            top_n: 10,
            symbol: Some("bridge".to_string()),
            file: None,
        },
    )
    .expect("bridges lookup ok");

    assert_eq!(resp.bridges.len(), 1, "lookup returns single entry");
    assert_eq!(resp.bridges[0].name, "bridge");
    assert!(resp.meta.target_found);
    assert_eq!(
        resp.meta.target_rank,
        Some(1),
        "bridge dominates dumbbell → rank 1"
    );
}

#[test]
fn lookup_unknown_symbol_returns_suggestions() {
    // AS-018 — empty bridges + Levenshtein suggestions.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("e.py"),
        "def linker():\n    return 0\n\ndef caller():\n    linker()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(
        &store,
        &BridgesRequest {
            top_n: 10,
            symbol: Some("nonexistent_zzz_98765".to_string()),
            file: None,
        },
    )
    .expect("bridges lookup ok");

    assert!(resp.bridges.is_empty());
    assert!(!resp.meta.target_found);
    assert_eq!(resp.meta.target_rank, None);
    assert!(
        !resp.meta.suggestion.is_empty(),
        "expected at least one suggestion"
    );
}

#[test]
fn small_graph_does_not_sample() {
    // < SAMPLE_THRESHOLD (5000 nodes) → meta.sampled must be false.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("s.py"),
        "def a(): return b()\n\ndef b(): return c()\n\ndef c(): return 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = bridges(&store, &BridgesRequest::default()).expect("bridges ok");
    assert!(!resp.meta.sampled, "tiny graph must use exact algorithm");
    assert_eq!(resp.meta.sample_size, resp.meta.total_nodes);
}
