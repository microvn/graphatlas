//! `metadata.json` lifecycle: build sentinel, atomic commit, schema check.
//! Backs Foundation S-003 AS-007/AS-008/AS-025/AS-027.

use crate::cache::{verify_file_perms, write_file_0600, CacheLayout};
use crate::SCHEMA_VERSION;
use ga_core::{Error, IndexState, Lang, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// On-disk form. Superset of `ga_core::GraphMeta` — adds fields needed for
/// crash-recovery + cross-process coordination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub schema_version: u32,
    pub indexed_at: u64,
    #[serde(default)]
    pub committed_at: Option<u64>,
    pub repo_root: String,
    pub index_state: IndexState,
    pub index_generation: String,
    #[serde(default)]
    pub indexed_root_hash: String,
    /// v1.5 PR4 staleness Phase B — monotonic version counter.
    ///
    /// Bumped by 1 on every successful `commit_in_place` / `commit`. Used
    /// by RO connections to detect that a sibling writer committed new
    /// state (via `Store::reopen_if_stale`).
    ///
    /// Authoritative storage is the lbug `GraphMeta` row
    /// `(key="graph_generation", value="<u64>")` (per multi-voice challenge
    /// C-2 — bumped atomically with data inside one lbug transaction).
    /// This field is the on-disk mirror in metadata.json so RO peers can
    /// read the value without opening a lbug connection.
    ///
    /// Serde default `0` distinguishes "never committed" from "gen 1
    /// fresh build". First commit lifts to 1.
    #[serde(default)]
    pub graph_generation: u64,

    /// v1.2-php S-001 AS-019 — snapshot of engine-supported `Lang` set at
    /// the moment this cache was built. Used by [`cache_outdated_for_lang_set`]
    /// to detect upgrade / downgrade where the engine's supported langs
    /// diverge from what was indexed.
    ///
    /// Serde default `Vec::new()` distinguishes "v1.1-era cache (no
    /// fingerprint)" from "v1.2+ cache with explicit empty set". The
    /// empty case is treated as "unknown — assume mismatch when the repo
    /// has files matching any current engine lang".
    #[serde(default)]
    pub cache_lang_set: Vec<Lang>,
}

/// Decision made at cold-load time.
#[derive(Debug)]
pub enum SchemaDecision {
    /// No cache dir/file exists — indexer should fresh-build.
    NoCache,
    /// Cache matches binary schema and is `complete` — use it.
    Match(Metadata),
    /// Cache schema_version != binary — delete + rebuild (AS-008 / AS-027).
    Mismatch { cache: u32, binary: u32 },
    /// Cache is `building` → previous indexer crashed; delete + rebuild (AS-025).
    CrashedBuilding { generation: String },
}

impl Metadata {
    /// Write initial `building` sentinel. Called at start of indexing.
    pub fn begin_indexing(layout: &CacheLayout, repo_root: &str) -> Result<Self> {
        Self::begin_indexing_with_schema(layout, repo_root, SCHEMA_VERSION)
    }

    /// Same as [`begin_indexing`] but lets callers pin a non-default schema
    /// version — used by Store when the binary has bumped schema and wants the
    /// new metadata.json to carry the new version.
    pub fn begin_indexing_with_schema(
        layout: &CacheLayout,
        repo_root: &str,
        schema_version: u32,
    ) -> Result<Self> {
        let m = Self {
            schema_version,
            indexed_at: unix_now(),
            committed_at: None,
            repo_root: repo_root.to_string(),
            index_state: IndexState::Building,
            index_generation: Uuid::new_v4().to_string(),
            indexed_root_hash: String::new(),
            // graph_generation starts at 0 (sentinel "never committed").
            // commit/commit_in_place bumps to >=1 on first success.
            graph_generation: 0,
            // v1.2-php S-001 AS-019 — snapshot engine's full lang set so
            // future versions can detect upgrade gap.
            cache_lang_set: Lang::ALL.to_vec(),
        };
        m.write(layout)?;
        Ok(m)
    }

    /// Atomic transition `building → complete`. Overwrites metadata.json via
    /// tmp-file + rename so a crash never leaves a half-written commit.
    pub fn commit(mut self, layout: &CacheLayout) -> Result<Self> {
        self.index_state = IndexState::Complete;
        self.committed_at = Some(unix_now());
        self.write(layout)?;
        Ok(self)
    }

    /// Same atomic transition as [`commit`], but mutates in place instead of
    /// consuming. MCP session lifecycle (`prepare_store_for_mcp`) needs the
    /// metadata to flip to Complete while the Store stays alive for serving.
    pub fn commit_in_place(&mut self, layout: &CacheLayout) -> Result<()> {
        self.index_state = IndexState::Complete;
        self.committed_at = Some(unix_now());
        self.write(layout)
    }

    /// AS-007 entry point. Returns the right action for the caller.
    pub fn cold_load(layout: &CacheLayout, binary_schema: u32) -> Result<SchemaDecision> {
        let path = layout.metadata_json();
        if !path.exists() {
            return Ok(SchemaDecision::NoCache);
        }
        // Perms first (AS-029).
        verify_file_perms(&path)?;
        let m = Self::load_from(&path)?;
        if m.schema_version != binary_schema {
            return Ok(SchemaDecision::Mismatch {
                cache: m.schema_version,
                binary: binary_schema,
            });
        }
        if m.index_state == IndexState::Building {
            return Ok(SchemaDecision::CrashedBuilding {
                generation: m.index_generation,
            });
        }
        Ok(SchemaDecision::Match(m))
    }

    /// Read metadata.json unconditionally (for doctor / list / tests).
    pub fn load(layout: &CacheLayout) -> Result<Self> {
        Self::load_from(&layout.metadata_json())
    }

    fn load_from(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice::<Self>(&bytes).map_err(|e| Error::ConfigCorrupt {
            path: path.display().to_string(),
            reason: format!("metadata.json corrupt: {e}"),
        })
    }

    fn write(&self, layout: &CacheLayout) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| Error::ConfigCorrupt {
            path: layout.metadata_json().display().to_string(),
            reason: format!("serialize: {e}"),
        })?;
        write_file_0600(&layout.metadata_json(), &json)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Langs that existed before v1.2 introduced the `cache_lang_set` fingerprint.
/// An empty `cache.cache_lang_set` in a v1.1-era metadata.json is implicitly
/// equivalent to this set: v1.1 supported these 9, so the cache covers them
/// implicitly. Invalidation triggers when the repo has langs NOT in this
/// baseline (e.g., PHP added in v1.2).
const PRE_FINGERPRINT_BASELINE: &[Lang] = &[
    Lang::Python,
    Lang::TypeScript,
    Lang::JavaScript,
    Lang::Go,
    Lang::Rust,
    Lang::Java,
    Lang::Kotlin,
    Lang::CSharp,
    Lang::Ruby,
];

/// v1.2-php S-001 AS-019 — decide whether the cache is outdated relative to
/// the engine's current `Lang` support and the repo's actual language mix.
///
/// Returns `Some(reason)` when an invalidate-and-rebuild is required:
///   - **Upgrade gap**: engine supports langs not in `cache.cache_lang_set`
///     AND the repo contains files matching at least one of those new langs.
///     This is the common case (v1.1 → v1.2 with PHP files in repo).
///   - **Downgrade gap**: cache references langs the engine no longer
///     supports. Safer to rebuild than serve stale references.
///   - **Empty fingerprint + new-lang files**: v1.1-era caches have empty
///     `cache_lang_set` (serde default). Treated via [`PRE_FINGERPRINT_BASELINE`]
///     substitution — assume cache covered the v1.1 lang set; invalidate
///     only if repo has a v1.2+ lang.
///
/// Returns `None` when the cache is sufficient for serving the repo:
///   - cache_lang_set ⊇ engine.lang_set (no new langs added)
///   - cache_lang_set ⊊ engine.lang_set but the new langs aren't present
///     in this repo
///   - empty cache_lang_set + empty repo
pub fn cache_outdated_for_lang_set(
    cache: &Metadata,
    engine_langs: &[Lang],
    repo_langs: &[Lang],
) -> Option<String> {
    use std::collections::HashSet;

    // Empty cache_lang_set (v1.1-era cache) → substitute the documented
    // pre-fingerprint baseline as the implicit "what the cache covered".
    let effective_cache: HashSet<Lang> = if cache.cache_lang_set.is_empty() {
        PRE_FINGERPRINT_BASELINE.iter().copied().collect()
    } else {
        cache.cache_lang_set.iter().copied().collect()
    };
    let engine_set: HashSet<Lang> = engine_langs.iter().copied().collect();
    let repo_set: HashSet<Lang> = repo_langs.iter().copied().collect();

    // Downgrade — cache claims langs the engine dropped. Always invalidate
    // (cache may reference symbols / edges for dropped langs).
    let mut dropped: Vec<Lang> = effective_cache.difference(&engine_set).copied().collect();
    dropped.sort_by_key(|l| l.as_str());
    if !dropped.is_empty() {
        return Some(format!(
            "cache_lang_set_downgrade: cache built with langs {:?} that this engine no longer supports",
            dropped
        ));
    }

    // Upgrade gap — engine supports langs the cache (effective) doesn't.
    // Invalidate only if the repo has files for one of those new langs.
    let new_in_engine: HashSet<Lang> = engine_set.difference(&effective_cache).copied().collect();
    if new_in_engine.is_empty() {
        return None;
    }

    let mut trigger: Vec<Lang> = new_in_engine.intersection(&repo_set).copied().collect();
    trigger.sort_by_key(|l| l.as_str());
    if !trigger.is_empty() {
        let prefix = if cache.cache_lang_set.is_empty() {
            "cache_lang_set_pre_v1_2"
        } else {
            "cache_lang_set_upgrade"
        };
        return Some(format!(
            "{prefix}: engine added support for langs {:?} which exist in this repo — rebuild to index them",
            trigger
        ));
    }

    None
}
