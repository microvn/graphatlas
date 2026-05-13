//! S-003 AS-007 + AS-008 + AS-025 + AS-027 — metadata.json lifecycle.
//!
//! - AS-007 cold load: metadata.json read, schema_version matches → proceed.
//! - AS-008 / AS-027: schema_version mismatch → caller deletes + rebuilds.
//! - AS-025 crash recovery: index_state starts `building`, atomic rename to `complete`
//!   on commit; cold-load of `building` state signals rebuild required.

use ga_core::IndexState;
use ga_index::cache::CacheLayout;
use ga_index::metadata::{Metadata, SchemaDecision};
use std::path::Path;
use tempfile::TempDir;

fn layout(tmp: &TempDir, repo: &str) -> CacheLayout {
    let root = tmp.path().join(".graphatlas");
    CacheLayout::for_repo(&root, Path::new(repo))
}

#[test]
fn begin_indexing_writes_building_state() {
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/a");
    layout.ensure_dir().unwrap();
    let m = Metadata::begin_indexing(&layout, "/work/a").unwrap();
    assert_eq!(m.index_state, IndexState::Building);
    assert_eq!(m.schema_version, ga_index::SCHEMA_VERSION);
    assert!(
        !m.index_generation.is_empty(),
        "index_generation must be set"
    );
    assert!(m.indexed_at > 0);

    let on_disk = Metadata::load(&layout).unwrap();
    assert_eq!(on_disk.index_state, IndexState::Building);
    assert_eq!(on_disk.index_generation, m.index_generation);
}

#[test]
fn commit_atomic_transitions_to_complete() {
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/b");
    layout.ensure_dir().unwrap();
    let m = Metadata::begin_indexing(&layout, "/work/b").unwrap();
    let original_gen = m.index_generation.clone();
    m.commit(&layout).unwrap();

    let loaded = Metadata::load(&layout).unwrap();
    assert_eq!(loaded.index_state, IndexState::Complete);
    assert_eq!(loaded.index_generation, original_gen);
}

#[test]
fn cold_load_decision_matches_happy_path() {
    // AS-007: fresh complete cache with matching schema → Ok(Match).
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/c");
    layout.ensure_dir().unwrap();
    Metadata::begin_indexing(&layout, "/work/c")
        .unwrap()
        .commit(&layout)
        .unwrap();

    let decision = Metadata::cold_load(&layout, ga_index::SCHEMA_VERSION).unwrap();
    match decision {
        SchemaDecision::Match(m) => {
            assert_eq!(m.index_state, IndexState::Complete);
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

#[test]
fn cold_load_decision_schema_mismatch_signals_rebuild() {
    // AS-008: cache has an OLDER schema_version than binary → Ok(Mismatch).
    // Pin cache at 1 explicitly so the test stays stable across binary
    // schema bumps (Foundation-C15 bumped SCHEMA_VERSION 1 → 2).
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/d");
    layout.ensure_dir().unwrap();
    Metadata::begin_indexing_with_schema(&layout, "/work/d", 1)
        .unwrap()
        .commit(&layout)
        .unwrap();

    let decision = Metadata::cold_load(&layout, 999).unwrap();
    match decision {
        SchemaDecision::Mismatch { cache, binary } => {
            assert_eq!(cache, 1);
            assert_eq!(binary, 999);
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
}

#[test]
fn cold_load_decision_building_signals_rebuild() {
    // AS-025: indexer crashed mid-run → index_state=building → Ok(CrashedBuilding).
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/e");
    layout.ensure_dir().unwrap();
    Metadata::begin_indexing(&layout, "/work/e").unwrap();
    // deliberately do NOT commit → simulates crash.

    let decision = Metadata::cold_load(&layout, ga_index::SCHEMA_VERSION).unwrap();
    assert!(
        matches!(decision, SchemaDecision::CrashedBuilding { .. }),
        "expected CrashedBuilding, got {decision:?}"
    );
}

#[test]
fn cold_load_missing_cache_signals_fresh_build() {
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/f");
    layout.ensure_dir().unwrap();

    let decision = Metadata::cold_load(&layout, ga_index::SCHEMA_VERSION).unwrap();
    assert!(
        matches!(decision, SchemaDecision::NoCache),
        "expected NoCache, got {decision:?}"
    );
}

#[test]
fn cold_load_corrupt_metadata_is_error() {
    let tmp = TempDir::new().unwrap();
    let layout = layout(&tmp, "/work/g");
    layout.ensure_dir().unwrap();
    ga_index::cache::write_file_0600(&layout.metadata_json(), b"{this is not json").unwrap();

    let err = Metadata::cold_load(&layout, 1).unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("corrupt")
            || format!("{err}").to_lowercase().contains("config"),
        "err: {err}"
    );
}
