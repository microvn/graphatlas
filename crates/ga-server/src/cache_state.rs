//! Resolve a slug → cache state for the staleness middleware and the
//! Corrupt-503 gate (Spec A AS-040/AS-041, C-cross-8).
//!
//! Stays a thin module so handlers don't reach into `ga_index::metadata`
//! directly.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    Fresh,
    Building,
    Corrupt,
    /// Cache dir doesn't exist for the slug.
    NotFound,
}

/// Look up the cache dir for `slug` under `cache_root`, peek at
/// `metadata.json`, and classify the state. Corrupt = metadata flagged
/// `index_state: Corrupt` (post-cancel-reindex per C-cross-8) OR
/// metadata parse failure. Building = `IndexState::Building` sentinel.
/// NotFound = no dir whose name ends in slug.
pub fn lookup_cache_state(cache_root: &Path, slug: &str) -> CacheState {
    let Some(dir) = find_cache_dir(cache_root, slug) else {
        return CacheState::NotFound;
    };
    let md_path = dir.join("metadata.json");
    let Ok(bytes) = std::fs::read(&md_path) else {
        return CacheState::NotFound;
    };
    // Try strict parse via ga_index::metadata::Metadata first — the
    // canonical shape.
    if let Ok(md) = serde_json::from_slice::<ga_index::metadata::Metadata>(&bytes) {
        return match md.index_state {
            ga_core::IndexState::Building => CacheState::Building,
            ga_core::IndexState::Complete => CacheState::Fresh,
        };
    }
    // Fall back to a loose JSON probe for a string `index_state` field
    // that's set to "corrupt" by future cancel-reindex flow (the spec
    // says cache_state Corrupt; ga_core::IndexState only has
    // Building/Complete today — Corrupt is layered on by ga-server's
    // own state file when S-005 ships cancel handling).
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if v.get("index_state").and_then(|s| s.as_str()) == Some("corrupt") {
            return CacheState::Corrupt;
        }
    }
    CacheState::Corrupt
}

fn find_cache_dir(cache_root: &Path, slug: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(cache_root).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(slug) || name == slug {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn seed(root: &Path, dir_name: &str, body: &serde_json::Value) -> std::path::PathBuf {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("metadata.json"),
            serde_json::to_vec_pretty(body).unwrap(),
        )
        .unwrap();
        dir
    }

    fn complete_body() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 5,
            "indexed_at": 1u64,
            "committed_at": 1u64,
            "repo_root": "/x",
            "index_state": "complete",
            "index_generation": "g",
            "indexed_root_hash": "",
            "graph_generation": 1,
            "cache_lang_set": []
        })
    }

    #[test]
    fn fresh_when_complete() {
        let tmp = tempdir().unwrap();
        seed(tmp.path(), "x-abc123", &complete_body());
        assert_eq!(lookup_cache_state(tmp.path(), "abc123"), CacheState::Fresh);
    }

    #[test]
    fn building_when_sentinel() {
        let tmp = tempdir().unwrap();
        let mut b = complete_body();
        b["index_state"] = serde_json::json!("building");
        seed(tmp.path(), "x-build1", &b);
        assert_eq!(
            lookup_cache_state(tmp.path(), "build1"),
            CacheState::Building
        );
    }

    #[test]
    fn corrupt_when_state_marker_set() {
        let tmp = tempdir().unwrap();
        let mut b = complete_body();
        b["index_state"] = serde_json::json!("corrupt");
        seed(tmp.path(), "x-corp01", &b);
        assert_eq!(
            lookup_cache_state(tmp.path(), "corp01"),
            CacheState::Corrupt
        );
    }

    #[test]
    fn corrupt_when_metadata_unparseable() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("x-junk00");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("metadata.json"), b"not json").unwrap();
        assert_eq!(
            lookup_cache_state(tmp.path(), "junk00"),
            CacheState::Corrupt
        );
    }

    #[test]
    fn not_found_when_no_dir() {
        let tmp = tempdir().unwrap();
        assert_eq!(
            lookup_cache_state(tmp.path(), "nope01"),
            CacheState::NotFound
        );
    }
}
