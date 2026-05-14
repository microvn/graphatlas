//! Cache file removal + placeholder metadata helpers.
//!
//! Extracted from `store.rs`. `nuke_cache_files` is the GitNexus
//! "close → rm → reopen" pattern's middle step (PR6 + PR6.1a reuse this
//! to wipe graph.db / .wal / sidecars between consume and reopen of a
//! Store). `placeholder_metadata` is the sentinel used by `Store::commit`
//! when consuming the owned `Metadata` for an atomic transition.

use crate::cache::CacheLayout;
use crate::metadata::Metadata;

/// Remove `metadata.json`, `graph.db` (file form), `graph.db` (dir form),
/// and any sibling shadow / WAL files that share the `graph.db` stem.
///
/// Best-effort: ignored errors mirror the original behavior because
/// rebuild paths immediately reopen the cache after the call, and
/// recovery surfaces any remaining inconsistency.
pub(crate) fn nuke_cache_files(layout: &CacheLayout) {
    let _ = std::fs::remove_file(layout.metadata_json());
    // lbug graph.db can be file or dir depending on version/platform — try both.
    let db = layout.graph_db();
    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_dir_all(&db);
    // lbug WAL / shadow files share the same stem.
    if let Some(parent) = db.parent() {
        if let Some(stem) = db.file_name().and_then(|n| n.to_str()) {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for e in entries.flatten() {
                    if let Some(n) = e.file_name().to_str() {
                        if n.starts_with(stem) && n != stem {
                            let _ = std::fs::remove_file(e.path());
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
            }
        }
    }
}

/// Sentinel `Metadata` used by `Store::commit` to consume + replace the
/// owned metadata while keeping the Store struct alive for the duration
/// of the disk write.
pub(crate) fn placeholder_metadata() -> Metadata {
    Metadata {
        schema_version: 0,
        indexed_at: 0,
        committed_at: None,
        repo_root: String::new(),
        index_state: ga_core::IndexState::Building,
        index_generation: String::new(),
        indexed_root_hash: String::new(),
        graph_generation: 0,
        // Placeholder is consumed-and-replaced by commit; lang_set populated
        // by the real begin_indexing() metadata that replaces it.
        cache_lang_set: Vec::new(),
    }
}
