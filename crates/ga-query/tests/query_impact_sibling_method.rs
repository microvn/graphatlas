//! KG-9 Action 2 — sibling-method blast radius via CONTAINS reverse-forward.
//!
//! Pattern ported from rust-poc/src/main.rs:2217-2227:
//! `MATCH (seed)<-[:CONTAINS]-(cls)-[:CONTAINS]->(sib)-[:CALLS]->(t)`.
//! The main BFS (bfs.rs, CALLS ∪ REFERENCES from seed) cannot reach files
//! that only sibling methods call, because there is no direct graph edge
//! between the seed method and the sibling's callees. CONTAINS bridges
//! `seed → class → sibling`, and then one CALLS hop from the sibling
//! surfaces the co-located blast radius — key for OOP recall on django /
//! nest where changes to one class method should surface other methods'
//! effects as potentially-affected.
//!
//! Depth convention: sibling-discovered files land at depth=2 (rust-poc
//! uses `file_depth.entry(p).or_insert(2)` — insert-if-absent so direct-
//! BFS depths aren't overwritten).

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
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

#[test]
fn sibling_method_callees_surface_at_depth_two() {
    // Fixture:
    //   domain.py: class User { set_password() -> storage.save(); verify() -> auth.check() }
    //   storage.py: def save()
    //   auth.py:    def check()
    //
    // Seed = set_password. Direct CALLS BFS reaches storage.py (depth 1) only.
    // CONTAINS reverse-forward reaches auth.py (depth 2) via sibling `verify`.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("storage.py"),
        "# helper for set_password\ndef save():\n    pass\n",
    );
    write(
        &repo.join("auth.py"),
        "# sibling of set_password via verify\ndef check():\n    pass\n",
    );
    write(
        &repo.join("domain.py"),
        "from storage import save\n\
         from auth import check\n\n\
         class User:\n\
         \x20   def set_password(self):\n\
         \x20       save()\n\n\
         \x20   def verify(self):\n\
         \x20       check()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("set_password".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let paths: Vec<&str> = resp
        .impacted_files
        .iter()
        .map(|f| f.path.as_str())
        .collect();

    assert!(
        paths.contains(&"auth.py"),
        "auth.py must be surfaced via CONTAINS sibling-method traversal (seed=set_password, sibling verify calls check in auth.py); got {:?}",
        paths
    );

    // Depth of auth.py should be 2 (sibling pattern depth), NOT overwriting
    // any direct-BFS depth. storage.py — direct callee — should stay at
    // whatever the main BFS assigned (expected depth 1).
    let auth_depth = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "auth.py")
        .map(|f| f.depth);
    assert_eq!(
        auth_depth,
        Some(2),
        "sibling-discovered file should be depth=2"
    );
}

#[test]
fn sibling_method_query_does_not_surface_files_for_non_class_seed() {
    // Seed is a top-level function (no enclosing class). The reverse
    // pattern finds no CONTAINS match, so sibling traversal must
    // contribute zero files.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("helpers.py"),
        "# called from standalone\ndef other():\n    pass\n",
    );
    write(
        &repo.join("top.py"),
        "from helpers import other\n\n\
         def standalone():\n    other()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("standalone".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // Main BFS reaches helpers.py (direct callee). The test is: no SPURIOUS
    // extras pulled in by a misfiring CONTAINS query on a non-class seed.
    // Concretely: total file count equals what direct BFS would produce.
    let paths: Vec<&str> = resp
        .impacted_files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(paths.contains(&"top.py"));
    assert!(paths.contains(&"helpers.py"));
    assert_eq!(
        paths.len(),
        2,
        "no sibling-method pattern should fire for non-class seed; got {paths:?}"
    );
}

#[test]
fn sibling_pattern_does_not_lower_existing_direct_bfs_depth() {
    // Safety: if a file is reached at depth 1 by direct BFS AND also
    // reachable via sibling traversal (which would target depth 2), the
    // direct-BFS depth must win. rust-poc uses `or_insert(2)` so it
    // only writes when absent.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // shared.py is called BOTH by set_password directly and by verify
    // (sibling). Direct BFS assigns depth 1; sibling pattern would try
    // depth 2 on the same file.
    write(
        &repo.join("shared.py"),
        "# used from set_password and verify\ndef helper():\n    pass\n",
    );
    write(
        &repo.join("domain.py"),
        "from shared import helper\n\n\
         class User:\n\
         \x20   def set_password(self):\n\
         \x20       helper()\n\n\
         \x20   def verify(self):\n\
         \x20       helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("set_password".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let shared_depth = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "shared.py")
        .map(|f| f.depth);
    assert_eq!(
        shared_depth,
        Some(1),
        "shared.py must stay at BFS depth=1, not be overwritten to 2 by sibling pattern"
    );
}
