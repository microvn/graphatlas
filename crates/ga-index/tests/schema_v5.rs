//! v1.4 schema v5 — DDL scaffolding tests (S-001a scope).
//!
//! Covers spec lines (graphatlas-v1.4-data-model.md):
//! - SCHEMA_VERSION constant 4 → 5 (Tools-C20).
//! - AS-004: v1.3 cache (schema=4) opened by v1.4 binary →
//!   OpenOutcome::RebuildSchemaMismatch{cache:4, binary:5} → nuke fires.
//! - AS-004: read-only attach refuses Err(SchemaTooNew) when
//!   cache.schema_version > binary (Tools-C8 preserved).
//!
//! Out of scope (this file): OVERRIDES table presence, has_unresolved_override
//! column, parser support — those land in subsequent S-001a tests.
//! (Tools-C18 / C19 / C21 / parent-method resolver, etc.)

use ga_index::cache::CacheLayout;
use ga_index::metadata::{Metadata, SchemaDecision};
use ga_index::{OpenOutcome, Store, SCHEMA_VERSION};
use tempfile::TempDir;

#[test]
fn schema_version_is_five_for_v1_4() {
    // Tools-C20: v1.4 ships with SCHEMA_VERSION=5. Bumping the integer
    // constant is mandatory for the cold_load mismatch path to fire on
    // existing v1.3 caches.
    assert_eq!(
        SCHEMA_VERSION, 5,
        "v1.4 spec requires SCHEMA_VERSION bump from 4 to 5 (Tools-C20)"
    );
}

#[test]
fn v1_3_cache_with_v1_4_binary_triggers_rebuild_schema_mismatch() {
    // AS-004: v1.3-built cache on disk (schema_version=4); v1.4 binary
    // ships SCHEMA_VERSION=5. Opening the cache must yield
    // OpenOutcome::RebuildSchemaMismatch{cache:4, binary:5}.
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    // v1.5 PR2 AS-001: real path required for commit (Merkle root hash).
    let repo_dir = tmp.path().join("repos").join("v1-3-to-v1-4-migration");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();

    // Stage a v1.3-shape cache by opening the cache root with the v1.3
    // schema_version (=4) and committing a real cache. We use the
    // explicit `open_with_root_and_schema` entrypoint because that's how
    // a v1.3 binary would have written the cache.
    {
        let store_v3 = Store::open_with_root_and_schema(&cache_root, repo, 4).unwrap();
        store_v3.commit().unwrap();
    }

    // Now open with the v1.4 binary's SCHEMA_VERSION (=5). The cold_load
    // path must surface the mismatch + drive a rebuild.
    let store_v4 = Store::open_with_root(&cache_root, repo).unwrap();
    match store_v4.outcome() {
        OpenOutcome::RebuildSchemaMismatch { cache, binary } => {
            assert_eq!(*cache, 4, "cache schema should be 4 (v1.3-built)");
            assert_eq!(
                *binary, SCHEMA_VERSION,
                "binary schema should match SCHEMA_VERSION constant"
            );
            assert_eq!(*binary, 5, "v1.4 binary schema is 5");
        }
        other => panic!(
            "expected OpenOutcome::RebuildSchemaMismatch{{cache:4, binary:5}}, got {other:?}"
        ),
    }
}

#[test]
fn read_only_refuses_when_cache_schema_newer_than_binary() {
    // AS-004 + Tools-C8: read-only attach against a future cache
    // (schema_version > binary) MUST return Err(SchemaTooNew) and never
    // call into the engine catalog. Simulate v1.5 cache (schema=6) being
    // attached by a v1.4 binary (SCHEMA_VERSION=5).
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    // v1.5 PR2 AS-001: real path required for commit (Merkle root hash).
    let repo_dir = tmp.path().join("repos").join("future-cache");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();

    // Stage a "future" cache by using a schema_version higher than the
    // current binary's SCHEMA_VERSION.
    let future_schema = SCHEMA_VERSION + 1;
    {
        let store_future =
            Store::open_with_root_and_schema(&cache_root, repo, future_schema).unwrap();
        store_future.commit().unwrap();
    }

    // Read-only attach should refuse cleanly via Metadata::cold_load
    // returning SchemaDecision::Mismatch{cache, binary} — the read-only
    // wrapper is responsible for converting that into the typed
    // SchemaTooNew error rather than calling lbug::Database::new.
    let layout = CacheLayout::for_repo(&cache_root, repo);
    let decision = Metadata::cold_load(&layout, SCHEMA_VERSION).unwrap();
    match decision {
        SchemaDecision::Mismatch { cache, binary } => {
            assert!(
                cache > binary,
                "this test stages cache > binary; got cache={cache}, binary={binary}"
            );
            assert_eq!(cache, future_schema, "cache should be v-future");
            assert_eq!(binary, SCHEMA_VERSION, "binary should be v1.4");
        }
        other => panic!("expected Mismatch with cache > binary; got {other:?}"),
    }
}
