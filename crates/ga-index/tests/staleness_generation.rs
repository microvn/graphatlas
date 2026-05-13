//! v1.5 PR4 Staleness Phase B — graph_generation counter + reopen_if_stale.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-staleness.md`
//! S-001 (AS-001..003) + S-002 (AS-004..006).

use ga_index::metadata::Metadata;
use ga_index::Store;
use std::path::PathBuf;
use tempfile::TempDir;

fn real_repo(tmp: &TempDir, rel: &str) -> PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    p
}

// =====================================================================
// S-001 AS-001: Fresh build initializes graph_generation = 1
// =====================================================================

#[test]
fn as_001_fresh_build_initializes_graph_generation_to_one() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "fresh-gen");

    let mut store = Store::open_with_root(&cache_root, &repo).expect("open");
    // Pre-commit: generation may be 0 sentinel (begin_indexing).
    store.commit_in_place().expect("commit_in_place");

    // metadata.json mirror reflects gen 1.
    let on_disk = Metadata::load(store.layout()).expect("load metadata");
    assert_eq!(
        on_disk.graph_generation, 1,
        "fresh build must initialize graph_generation = 1, got {}",
        on_disk.graph_generation
    );
    // Store's cached generation matches the on-disk value.
    assert_eq!(
        store.metadata().graph_generation,
        1,
        "in-memory metadata.graph_generation must be 1 post-commit"
    );
}

// =====================================================================
// S-001 AS-002: Subsequent commit bumps generation
// =====================================================================

#[test]
fn as_002_subsequent_commit_bumps_generation() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "bump-gen");

    // First commit → gen 1.
    {
        let mut s1 = Store::open_with_root(&cache_root, &repo).unwrap();
        s1.commit_in_place().unwrap();
        assert_eq!(s1.metadata().graph_generation, 1);
    }

    // Reopen on Resumed path (same schema). The cached metadata carries
    // graph_generation=1 from the first commit. A re-commit on the
    // Resumed Store must bump 1 → 2 (a writer process pulled fresh data
    // and called commit_in_place again — common reindex tool pattern).
    {
        let mut s2 = Store::open_with_root(&cache_root, &repo).unwrap();
        // Sanity: Resumed metadata carries the prior generation.
        assert_eq!(
            s2.metadata().graph_generation,
            1,
            "Resumed open must carry forward gen=1 from disk"
        );
        s2.commit_in_place().expect("second commit");
        assert_eq!(
            s2.metadata().graph_generation,
            2,
            "second commit must bump generation 1 → 2, got {}",
            s2.metadata().graph_generation
        );
        let on_disk = Metadata::load(s2.layout()).unwrap();
        assert_eq!(on_disk.graph_generation, 2);
    }
}

// =====================================================================
// S-001 AS-003: Generation persists across process boundary
// =====================================================================

#[test]
fn as_003_generation_persists_across_process_boundary() {
    // Simulate "process A commits gen N; process B opens same cache".
    // Same-process test using sequential Store instances against shared
    // TempDir — the Drop of `s1` flushes metadata, then s2 reads it back.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "persist-gen");

    let final_gen = {
        let mut s1 = Store::open_with_root(&cache_root, &repo).unwrap();
        s1.commit_in_place().unwrap();
        let gen = s1.metadata().graph_generation;
        drop(s1);
        gen
    };
    assert_eq!(final_gen, 1, "first commit gen");

    // Second instance — same cache. Outcome should be Resumed since the
    // first instance committed cleanly. graph_generation must round-trip
    // through metadata.json.
    let s2 = Store::open_with_root(&cache_root, &repo).unwrap();
    assert_eq!(
        s2.metadata().graph_generation,
        final_gen,
        "process boundary must preserve graph_generation; expected {final_gen}, got {}",
        s2.metadata().graph_generation
    );
}

// =====================================================================
// S-002 AS-004: reopen_if_stale no-op when generation matches
// =====================================================================

#[test]
fn as_004_reopen_if_stale_no_op_when_generation_matches() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "no-reopen");

    let mut s = Store::open_with_root(&cache_root, &repo).unwrap();
    s.commit_in_place().unwrap();
    // After commit the Store has cached_generation = 1 and metadata.json
    // also reports 1. reopen_if_stale should return Ok(false).
    let reopened = s.reopen_if_stale().expect("reopen_if_stale ok");
    assert!(
        !reopened,
        "reopen_if_stale must return false when generation matches"
    );
}

// =====================================================================
// S-002 AS-005: Bumped generation → reopen + new view
// =====================================================================

#[test]
fn as_005_reopen_if_stale_returns_true_when_generation_bumped() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "reopen-bump");

    // Process A: commit gen 1 + seal.
    let mut s = Store::open_with_root(&cache_root, &repo).unwrap();
    s.commit_in_place().unwrap();
    assert_eq!(s.metadata().graph_generation, 1);

    // Simulate "another writer bumped the on-disk metadata to gen 2" by
    // directly editing metadata.json. (In production this is what the
    // sibling MCP writer process would do via its own commit_in_place.)
    let mut on_disk = Metadata::load(s.layout()).unwrap();
    on_disk.graph_generation = 2;
    {
        // Reuse the same `write_file_0600` helper indirectly via
        // commit_in_place style: bypass by writing JSON directly.
        let json = serde_json::to_vec_pretty(&on_disk).unwrap();
        let mp = s.layout().metadata_json();
        std::fs::write(&mp, &json).unwrap();
    }

    // reopen_if_stale must detect the bump.
    let reopened = s.reopen_if_stale().expect("reopen_if_stale ok");
    assert!(
        reopened,
        "reopen_if_stale must return true when on-disk generation > cached"
    );
    assert_eq!(
        s.metadata().graph_generation,
        2,
        "cached_generation must update to 2 post-reopen"
    );
}

// =====================================================================
// S-002 AS-006: Reopen failure propagates (poisoned state)
// =====================================================================

#[test]
fn as_006_reopen_if_stale_returns_err_on_underlying_failure() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "reopen-fail");

    let mut s = Store::open_with_root(&cache_root, &repo).unwrap();
    s.commit_in_place().unwrap();

    // Bump generation on disk + corrupt metadata.json so reopen fails.
    // Easiest reproducible failure: corrupt the JSON.
    let mp = s.layout().metadata_json();
    std::fs::write(&mp, b"{not json").unwrap();

    let result = s.reopen_if_stale();
    assert!(
        result.is_err(),
        "reopen_if_stale must propagate Err when metadata corrupt"
    );
}
