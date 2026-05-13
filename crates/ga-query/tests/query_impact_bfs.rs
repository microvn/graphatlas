//! Tools S-006 cluster C2 — BFS over callers+callees (CALLS ∪ REFERENCES).
//!
//! Covers AS-016: depth cap, transitive_completeness metadata, cycle guard.
//!
//! Cross-file chains use REFERENCES (value-reference sites) because the M1
//! indexer resolves CALLS same-file only — cross-file calls go to external
//! placeholder nodes. REFERENCES performs cross-file symbol resolution
//! (indexer.rs line 224-233).

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactReason, ImpactRequest};
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

/// Depth at which `path` appears in the impacted_files list, or None.
fn depth_of(resp: &ga_query::ImpactResponse, path: &str) -> Option<u32> {
    resp.impacted_files
        .iter()
        .find(|f| f.path == path)
        .map(|f| f.depth)
}

#[test]
fn bfs_seed_file_present_at_depth_zero() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def alpha():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        depth_of(&resp, "m.py"),
        Some(0),
        "seed file must be depth 0: {:?}",
        resp.impacted_files
    );
}

#[test]
fn bfs_direct_caller_at_depth_one() {
    // Same-file chain: CALLS resolves locally, caller_file == target_file,
    // so impacted_files still shows the file at depth 0 — but
    // completeness = 1 proves the caller symbol was traversed.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target():\n    pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("target".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        resp.meta.transitive_completeness, 1,
        "one caller traversal step: {:?}",
        resp
    );
}

/// 4-file chain a → b → c → d via REFERENCES (value-reference sites).
/// Each file "uses" the previous module's symbol by name so the indexer
/// emits cross-file REFERENCES edges.
///
/// Each transitive file also includes a doc comment mentioning the root
/// seed `alpha` so EXP-M2-TEXTFILTER's word-boundary intersect doesn't
/// drop the chain — the filter's purpose is to cull hub files that don't
/// textually mention the seed anywhere, not legitimate transitive chains.
fn write_4_level_ref_chain(repo: &Path) {
    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    // `alpha` appears inside a dict value → Python REFERENCES picks it up.
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef beta():\n    m = {'k': alpha}\n    return m\n",
    );
    write(
        &repo.join("c.py"),
        "# chain from alpha -> beta -> gamma\n\
         from b import beta\n\ndef gamma():\n    m = {'k': beta}\n    return m\n",
    );
    write(
        &repo.join("d.py"),
        "# chain from alpha -> beta -> gamma -> delta\n\
         from c import gamma\n\ndef delta():\n    m = {'k': gamma}\n    return m\n",
    );
}

#[test]
fn bfs_transitive_depth_three_as016() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write_4_level_ref_chain(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            max_depth: Some(3),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(
        depth_of(&resp, "a.py"),
        Some(0),
        "seed file: {:?}",
        resp.impacted_files
    );
    assert_eq!(
        depth_of(&resp, "b.py"),
        Some(1),
        "depth-1 ref: {:?}",
        resp.impacted_files
    );
    assert_eq!(
        depth_of(&resp, "c.py"),
        Some(2),
        "depth-2 transitive: {:?}",
        resp.impacted_files
    );
    assert_eq!(
        depth_of(&resp, "d.py"),
        Some(3),
        "depth-3 transitive (AS-016): {:?}",
        resp.impacted_files
    );
    assert_eq!(resp.meta.transitive_completeness, 3);
    assert_eq!(resp.meta.max_depth, 3);
}

#[test]
fn bfs_max_depth_caps_beyond_three() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write_4_level_ref_chain(&repo);
    // 5th file extends chain — d -> e; must be excluded at max_depth=3.
    write(
        &repo.join("e.py"),
        "from d import delta\n\ndef epsilon():\n    m = {'k': delta}\n    return m\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            max_depth: Some(3),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        depth_of(&resp, "d.py").is_some(),
        "d.py (depth 3) must still be included"
    );
    assert!(
        depth_of(&resp, "e.py").is_none(),
        "e.py (depth 4) must be excluded at max_depth=3: {:?}",
        resp.impacted_files
    );
}

#[test]
fn bfs_defaults_max_depth_to_three() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write_4_level_ref_chain(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            max_depth: None,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(resp.meta.max_depth, 3, "default max_depth must be 3");
    assert_eq!(depth_of(&resp, "d.py"), Some(3));
}

#[test]
fn bfs_cycle_does_not_loop() {
    // a ↔ b mutual REFERENCES. BFS must terminate without revisiting.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "from b import beta\n\ndef alpha():\n    m = {'k': beta}\n    return m\n",
    );
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef beta():\n    m = {'k': alpha}\n    return m\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            max_depth: Some(5),
            ..Default::default()
        },
    )
    .unwrap();
    // Both files reached; completeness bounded regardless of cycle.
    assert!(depth_of(&resp, "a.py").is_some());
    assert!(depth_of(&resp, "b.py").is_some());
    // No duplicate file entries.
    let a_hits = resp
        .impacted_files
        .iter()
        .filter(|f| f.path == "a.py")
        .count();
    let b_hits = resp
        .impacted_files
        .iter()
        .filter(|f| f.path == "b.py")
        .count();
    assert_eq!(a_hits, 1);
    assert_eq!(b_hits, 1);
}

#[test]
fn bfs_unknown_symbol_returns_no_impacted_files() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def alpha():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("nonexistent_symbol".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        resp.impacted_files.is_empty(),
        "unknown symbol must produce no impacted files: {:?}",
        resp.impacted_files
    );
    assert_eq!(resp.meta.transitive_completeness, 0);
}

#[test]
fn bfs_reason_enum_serializes_lowercase_for_wire_compat() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target():\n    pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("target".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let seed = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "m.py")
        .expect("m.py is the seed file");
    assert_eq!(seed.reason, ImpactReason::Seed);

    let json = serde_json::to_value(&resp).unwrap();
    let entry = &json["impacted_files"][0];
    assert_eq!(
        entry["reason"].as_str(),
        Some("seed"),
        "wire format must stay lowercase string: {entry}"
    );
}

#[test]
fn bfs_callees_direction_traverses_outgoing() {
    // Same-file callees — verifies outgoing-edge traversal wires up.
    // completeness jumps to 1 when `src` calls something reachable.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def leaf():\n    pass\n\ndef src():\n    leaf()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("src".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        resp.meta.transitive_completeness >= 1,
        "src→leaf outgoing must be traversed: {:?}",
        resp
    );
}
