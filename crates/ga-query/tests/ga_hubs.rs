//! `ga_hubs` — degree-rank smoke tests on a hand-built mini-fixture.
//!
//! Goal: verify ordering by total_degree DESC and that excluded-by-design
//! signals (external symbols, zero-degree symbols) don't pollute the list.

use ga_index::Store;
use ga_query::hubs::{hubs, HubsRequest};
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
fn most_called_symbol_ranks_first() {
    let (_tmp, cache, repo) = setup();
    // `hot` is called 3x, `warm` 1x, `cold` 0x → expect hot > warm.
    // Cold appears in symbols but should be filtered out (zero edges).
    write(
        &repo.join("a.py"),
        "def hot():\n    return 1\n\n\
         def warm():\n    return 2\n\n\
         def cold():\n    return 3\n\n\
         def caller_a():\n    hot(); hot()\n\n\
         def caller_b():\n    hot(); warm()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 10,
            ..Default::default()
        },
    )
    .expect("hubs ok");
    let names: Vec<&str> = resp.hubs.iter().map(|h| h.name.as_str()).collect();

    // hot must be ranked above warm.
    let hot_pos = names.iter().position(|&n| n == "hot");
    let warm_pos = names.iter().position(|&n| n == "warm");
    assert!(hot_pos.is_some(), "hot should be present; got {names:?}");
    assert!(warm_pos.is_some(), "warm should be present; got {names:?}");
    assert!(
        hot_pos < warm_pos,
        "hot ({hot_pos:?}) must rank above warm ({warm_pos:?}); got {names:?}"
    );
    // cold has 0 in/out — must NOT appear.
    assert!(
        !names.contains(&"cold"),
        "zero-degree `cold` must be filtered; got {names:?}"
    );
}

#[test]
fn top_n_caps_response() {
    let (_tmp, cache, repo) = setup();
    // Build 20 symbols, each calling all the others (so each has high degree).
    let mut src = String::new();
    for i in 0..20 {
        src.push_str(&format!("def f{i}():\n    return {i}\n\n"));
    }
    src.push_str("def driver():\n");
    for i in 0..20 {
        src.push_str(&format!("    f{i}()\n"));
    }
    write(&repo.join("m.py"), &src);

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 5,
            ..Default::default()
        },
    )
    .expect("hubs ok");
    assert_eq!(resp.hubs.len(), 5, "top_n=5 must cap to 5 entries");
    assert!(
        resp.meta.truncated,
        "21+ candidates → truncated must be true"
    );
    assert!(
        resp.meta.total_symbols_with_edges >= 20,
        "expect ≥ 20 symbols with edges; got {}",
        resp.meta.total_symbols_with_edges
    );
}

#[test]
fn empty_repo_returns_empty_hubs() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("empty.py"), "# nothing here\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(&store, &HubsRequest::default()).expect("hubs ok");
    assert!(resp.hubs.is_empty(), "empty repo → empty hubs");
    assert_eq!(resp.meta.total_symbols_with_edges, 0);
    assert!(!resp.meta.truncated);
}

// ─── S-004 — symbol-lookup mode ──────────────────────────────────────

#[test]
fn lookup_returns_rank_for_known_symbol() {
    // AS-014 — engine returns the matching entry + 1-based rank against
    // the FULL sorted vec (not the truncated top-N view).
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "def hot():\n    return 1\n\n\
         def warm():\n    return 2\n\n\
         def caller_a():\n    hot(); warm()\n\n\
         def caller_b():\n    hot()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 10,
            symbol: Some("hot".to_string()),
            edge_types: ga_query::hubs::HubsEdgeTypes::Default,
            file: None,
        },
    )
    .expect("hubs lookup ok");

    assert_eq!(resp.hubs.len(), 1, "lookup returns single entry");
    assert_eq!(resp.hubs[0].name, "hot");
    assert!(resp.meta.target_found);
    assert_eq!(
        resp.meta.target_rank,
        Some(1),
        "hot is most-called → rank 1"
    );
    assert!(resp.meta.suggestion.is_empty());
}

#[test]
fn lookup_unknown_symbol_returns_suggestions() {
    // AS-015 — empty hubs + Levenshtein top-3 + target_found=false.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "def hot():\n    return 1\n\n\
         def warm():\n    return 2\n\n\
         def caller():\n    hot(); warm()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 10,
            symbol: Some("nonexistent_xyz_12345".to_string()),
            edge_types: ga_query::hubs::HubsEdgeTypes::Default,
            file: None,
        },
    )
    .expect("hubs lookup ok");

    assert!(resp.hubs.is_empty(), "unknown symbol → empty hubs");
    assert!(!resp.meta.target_found);
    assert_eq!(resp.meta.target_rank, None);
    // suggest_similar pulls from Symbol table — names that exist in the repo
    // should appear (even if Levenshtein distance is large).
    assert!(
        !resp.meta.suggestion.is_empty(),
        "expected at least one suggestion from {:?}",
        resp.meta.suggestion
    );
}

#[test]
fn lookup_with_file_disambiguates_same_name() {
    // AS-016 — two files define `helper`; file=b.py picks the b.py version.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "def helper():\n    return 1\n\n\
         def caller_in_a():\n    helper()\n",
    );
    write(
        &repo.join("b.py"),
        "def helper():\n    return 2\n\n\
         def caller_in_b():\n    helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 10,
            symbol: Some("helper".to_string()),
            edge_types: ga_query::hubs::HubsEdgeTypes::Default,
            file: Some("b.py".to_string()),
        },
    )
    .expect("hubs lookup ok");

    assert_eq!(resp.hubs.len(), 1);
    assert_eq!(resp.hubs[0].name, "helper");
    assert_eq!(resp.hubs[0].file, "b.py", "file filter must pick b.py");
    assert!(resp.meta.target_found);
}

#[test]
fn entry_carries_in_out_total_degrees() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "def target():\n    return 0\n\n\
         def caller_a():\n    target()\n\n\
         def caller_b():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = hubs(&store, &HubsRequest::default()).expect("hubs ok");
    let target = resp
        .hubs
        .iter()
        .find(|h| h.name == "target")
        .expect("target entry present");
    // Indexer dedupes by (caller, callee) pair → 2 distinct callers = 2 in.
    assert!(
        target.in_degree >= 2,
        "target called by 2 distinct callers; got {target:?}"
    );
    assert_eq!(
        target.total_degree,
        target.in_degree + target.out_degree,
        "total = in + out invariant"
    );
    assert!(!target.kind.is_empty(), "kind must be populated");
    assert!(target.line > 0, "line must be set");
    assert_eq!(target.file, "a.py");
}
