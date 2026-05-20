//! ga-ui Spec A S-003 — metadata.json migration.
//!
//! Covers:
//!   AS-030 cache cũ (no index_counts / no health_summary) parse OK
//!          → fields = None, no panic.
//!   AS-031 lbug schema unchanged — verified at the sidecar level: this
//!          test only touches metadata.json, never opens a lbug DB.
//!   set_index_counts / set_health_summary persist via atomic write,
//!          re-read round-trips correctly.

use ga_core::{HealthSummary, IndexCounts};
use ga_index::cache::CacheLayout;
use ga_index::metadata::Metadata;
use tempfile::tempdir;

fn layout(root: &std::path::Path) -> CacheLayout {
    // CacheLayout::for_repo(cache_root, repo_root) builds the per-repo
    // dir under cache_root using the `<name>-<6hex>` shape
    // `list_caches` discovers (Foundation-C12).
    CacheLayout::for_repo(root, std::path::Path::new("/tmp/spec-s003-fixture"))
}

// ---------- AS-030 ----------

#[test]
fn as030_pre_migration_cache_parses_with_none_fields() {
    let tmp = tempdir().unwrap();
    let lay = layout(tmp.path());
    std::fs::create_dir_all(lay.dir()).unwrap();

    // Hand-write a v1.5-era metadata.json (no index_counts/health_summary).
    let body = serde_json::json!({
        "schema_version": 5,
        "indexed_at": 1715712000_u64,
        "committed_at": 1715712001_u64,
        "repo_root": "/tmp/spec-s003-fixture",
        "index_state": "complete",
        "index_generation": "old-generation-uuid",
        "indexed_root_hash": "deadbeef",
        "graph_generation": 1,
        "cache_lang_set": []
        // <-- intentionally omit index_counts + health_summary
    });
    std::fs::write(
        lay.dir().join("metadata.json"),
        serde_json::to_vec_pretty(&body).unwrap(),
    )
    .unwrap();

    // Cold-load via the public read path.
    let bytes = std::fs::read(lay.dir().join("metadata.json")).unwrap();
    let m: Metadata = serde_json::from_slice(&bytes).expect("old metadata must still parse");
    assert!(m.index_counts.is_none(), "pre-migration → index_counts None");
    assert!(m.health_summary.is_none(), "pre-migration → health_summary None");
    assert_eq!(m.schema_version, 5);
    assert_eq!(m.repo_root, "/tmp/spec-s003-fixture");
}

// ---------- set_index_counts persists + round-trips ----------

#[test]
fn set_index_counts_persists_and_round_trips() {
    let tmp = tempdir().unwrap();
    let lay = layout(tmp.path());
    std::fs::create_dir_all(lay.dir()).unwrap();

    let mut m = Metadata::begin_indexing(&lay, "/tmp/spec-s003-fixture").unwrap();

    let counts = IndexCounts {
        node_count: 9876,
        edge_count: 23456,
        file_count: 312,
        last_index_duration_ms: 12450,
        db_size_bytes: 12_582_912,
    };
    m.set_index_counts(counts.clone(), &lay).unwrap();

    // Re-read the file fresh — verify persistence, not just in-memory mutation.
    let bytes = std::fs::read(lay.dir().join("metadata.json")).unwrap();
    let reread: Metadata = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reread.index_counts, Some(counts));
    // health_summary still untouched
    assert!(reread.health_summary.is_none());
}

// ---------- set_health_summary persists + round-trips ----------

#[test]
fn set_health_summary_persists_and_round_trips() {
    let tmp = tempdir().unwrap();
    let lay = layout(tmp.path());
    std::fs::create_dir_all(lay.dir()).unwrap();

    let mut m = Metadata::begin_indexing(&lay, "/tmp/spec-s003-fixture").unwrap();

    let summary = HealthSummary {
        computed_at_unix: 1_715_712_345,
        hubs_count: 42,
        bridges_count: 15,
        dead_code_count: 128,
        large_functions_count: 23,
        tested_count: 456,
    };
    m.set_health_summary(summary.clone(), &lay).unwrap();

    let bytes = std::fs::read(lay.dir().join("metadata.json")).unwrap();
    let reread: Metadata = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reread.health_summary, Some(summary));
    assert!(reread.index_counts.is_none());
}

// ---------- both set together survive a fresh deserialize ----------

#[test]
fn both_fields_coexist_in_persisted_metadata() {
    let tmp = tempdir().unwrap();
    let lay = layout(tmp.path());
    std::fs::create_dir_all(lay.dir()).unwrap();

    let mut m = Metadata::begin_indexing(&lay, "/tmp/spec-s003-fixture").unwrap();
    let counts = IndexCounts {
        node_count: 1,
        edge_count: 2,
        file_count: 3,
        last_index_duration_ms: 4,
        db_size_bytes: 5,
    };
    let summary = HealthSummary {
        computed_at_unix: 1,
        hubs_count: 2,
        bridges_count: 3,
        dead_code_count: 4,
        large_functions_count: 5,
        tested_count: 6,
    };
    m.set_index_counts(counts.clone(), &lay).unwrap();
    m.set_health_summary(summary.clone(), &lay).unwrap();

    let bytes = std::fs::read(lay.dir().join("metadata.json")).unwrap();
    let reread: Metadata = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reread.index_counts, Some(counts));
    assert_eq!(reread.health_summary, Some(summary));
}
