//! v1.2-php S-001 AS-019 — cache_lang_set fingerprint reindex invalidation.
//!
//! Detects upgrade across GA versions when the engine's supported `Lang` set
//! diverges from what was supported at index time. Without this, a v1.1 cache
//! (no PHP) on a Laravel repo upgraded to v1.2 silently returns empty PHP
//! results forever — confused user, wrong code changes.
//!
//! Test surface:
//! - `Metadata.cache_lang_set: Vec<Lang>` field (serde default empty for
//!   v1.1 compat — v1.1 metadata.json files deserialize cleanly).
//! - `begin_indexing_with_schema` populates `cache_lang_set` with `Lang::ALL`
//!   at index time.
//! - `cache_outdated_for_lang_set(cache, engine, repo_langs) -> Option<String>`
//!   returns Some("reason") when invalidation is needed.

use ga_core::Lang;
use ga_index::metadata::{cache_outdated_for_lang_set, Metadata};

/// Build a Metadata stub with the given cache_lang_set. Other fields
/// irrelevant for the lang-set check.
fn meta_with_lang_set(cache_langs: Vec<Lang>) -> Metadata {
    Metadata {
        schema_version: 0,
        indexed_at: 0,
        committed_at: None,
        repo_root: String::new(),
        index_state: ga_core::IndexState::Complete,
        index_generation: String::new(),
        indexed_root_hash: String::new(),
        graph_generation: 1,
        cache_lang_set: cache_langs,
    }
}

#[test]
fn cache_set_matches_engine_no_invalidation() {
    // Cache was indexed with full current engine support; nothing to do.
    let cache = meta_with_lang_set(Lang::ALL.to_vec());
    let engine = Lang::ALL;
    let repo = &[Lang::Python, Lang::Php];
    assert!(
        cache_outdated_for_lang_set(&cache, engine, repo).is_none(),
        "cache==engine MUST NOT trigger invalidation"
    );
}

#[test]
fn empty_cache_lang_set_with_php_repo_triggers_invalidation() {
    // Simulates v1.1 cache (no cache_lang_set field — serde default empty)
    // on a repo containing .php files. Engine now supports Lang::Php.
    let cache = meta_with_lang_set(Vec::new());
    let engine = Lang::ALL;
    let repo = &[Lang::Php];
    let result = cache_outdated_for_lang_set(&cache, engine, repo);
    assert!(
        result.is_some(),
        "v1.1 cache (empty cache_lang_set) + repo with .php MUST invalidate"
    );
    let reason = result.unwrap();
    assert!(
        reason.to_lowercase().contains("php") || reason.contains("Php"),
        "reason should mention the lang that triggered: '{reason}'"
    );
}

#[test]
fn empty_cache_lang_set_with_no_new_lang_files_does_not_invalidate() {
    // v1.1 cache + repo contains only v1.1-era langs (Python). Even though
    // cache_lang_set is empty, no new-lang files exist so the cache is still
    // useful — no invalidation needed.
    let cache = meta_with_lang_set(Vec::new());
    let engine = Lang::ALL;
    let repo = &[Lang::Python];
    assert!(
        cache_outdated_for_lang_set(&cache, engine, repo).is_none(),
        "no new-lang files in repo → cache is still useful"
    );
}

#[test]
fn proper_subset_with_missing_lang_in_repo_invalidates() {
    // Cache indexed pre-PHP (subset of v1.1 langs); repo now has .php.
    let cache_langs: Vec<Lang> = Lang::ALL
        .iter()
        .filter(|l| **l != Lang::Php)
        .copied()
        .collect();
    let cache = meta_with_lang_set(cache_langs);
    let engine = Lang::ALL;
    let repo = &[Lang::Php, Lang::Python];
    let result = cache_outdated_for_lang_set(&cache, engine, repo);
    assert!(
        result.is_some(),
        "cache⊊engine + repo has missing lang → invalidate; got {result:?}"
    );
}

#[test]
fn proper_subset_without_missing_lang_in_repo_does_not_invalidate() {
    // Cache indexed pre-PHP; repo has only Python files. Nothing PHP-shaped
    // means the missing lang doesn't affect THIS repo's coverage.
    let cache_langs: Vec<Lang> = Lang::ALL
        .iter()
        .filter(|l| **l != Lang::Php)
        .copied()
        .collect();
    let cache = meta_with_lang_set(cache_langs);
    let engine = Lang::ALL;
    let repo = &[Lang::Python];
    assert!(
        cache_outdated_for_lang_set(&cache, engine, repo).is_none(),
        "missing lang not present in repo → cache still useful"
    );
}

#[test]
fn cache_superset_of_engine_invalidates_downgrade() {
    // User downgraded GA → engine no longer supports a lang the cache
    // claims. Invalidate to avoid serving stale data referencing dropped lang.
    let cache_langs = Lang::ALL.to_vec();
    // Engine simulates downgrade — Php removed.
    let engine: Vec<Lang> = Lang::ALL
        .iter()
        .filter(|l| **l != Lang::Php)
        .copied()
        .collect();
    let cache = meta_with_lang_set(cache_langs);
    let repo = &[Lang::Python];
    let result = cache_outdated_for_lang_set(&cache, &engine, repo);
    assert!(
        result.is_some(),
        "downgrade (cache⊋engine) MUST invalidate; got {result:?}"
    );
}

#[test]
fn empty_cache_lang_set_empty_repo_does_not_invalidate() {
    // Edge case: v1.1 cache + completely empty repo (no lang detected).
    // Don't invalidate — there's nothing to re-index anyway.
    let cache = meta_with_lang_set(Vec::new());
    let engine = Lang::ALL;
    let repo: &[Lang] = &[];
    assert!(
        cache_outdated_for_lang_set(&cache, engine, repo).is_none(),
        "empty cache_lang_set + empty repo → no action"
    );
}

#[test]
fn v1_1_metadata_json_deserializes_without_cache_lang_set_field() {
    // Real concern: existing v1.1 metadata.json files in the wild MUST still
    // deserialize after we add the new field. Serde default = empty Vec.
    let v1_1_json = r#"{
        "schema_version": 5,
        "indexed_at": 1700000000,
        "committed_at": 1700000010,
        "repo_root": "/repo",
        "index_state": "complete",
        "index_generation": "abc-123",
        "indexed_root_hash": "deadbeef",
        "graph_generation": 7
    }"#;
    let m: Metadata = serde_json::from_str(v1_1_json).expect("v1.1 metadata.json must still parse");
    assert!(
        m.cache_lang_set.is_empty(),
        "missing field defaults to empty: {:?}",
        m.cache_lang_set
    );
}

#[test]
fn newly_built_cache_populates_cache_lang_set_with_engine_all() {
    use ga_index::cache::CacheLayout;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().to_path_buf();
    let repo_root = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_root).unwrap();
    let layout = CacheLayout::for_repo(&cache_root, &repo_root);
    // CacheLayout::for_repo derives a per-repo subdir; ensure it exists with 0700.
    std::fs::create_dir_all(layout.metadata_json().parent().unwrap()).unwrap();
    std::fs::set_permissions(
        layout.metadata_json().parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let m = Metadata::begin_indexing(&layout, "/repo").expect("begin_indexing OK");

    assert_eq!(
        m.cache_lang_set.len(),
        Lang::ALL.len(),
        "new cache MUST snapshot engine's full Lang::ALL set: {:?}",
        m.cache_lang_set
    );
    for &lang in Lang::ALL {
        assert!(
            m.cache_lang_set.contains(&lang),
            "cache_lang_set missing {lang:?}: {:?}",
            m.cache_lang_set
        );
    }
}
