//! S-005 AS-014 — per-file change detection.
//!
//! Given a known hash map `{rel_path → BLAKE3}` (typically read from the graph
//! cache), walk the repo and classify each file into added / modified /
//! unchanged / deleted buckets. The re-parse step (Tools phase) consumes
//! `added + modified`, the graph-delete step consumes `deleted`.

use crate::walk::walk_repo;
use ga_core::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ChangeSet {
    pub added: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
}

/// BLAKE3-hash a single file on disk. Streaming to stay constant-memory on
/// large inputs (though our size cap is 2MB, this keeps us honest).
pub fn file_blake3(path: &Path) -> Result<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::Other(anyhow::anyhow!("open {} failed: {e}", path.display())))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| Error::Other(anyhow::anyhow!("read {} failed: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Classify every source file under `repo_root` against `known` hashes.
/// Returns a [`ChangeSet`] with exclusive buckets (one file → one bucket).
pub fn detect_changed_files(
    repo_root: &Path,
    known: &HashMap<PathBuf, [u8; 32]>,
) -> Result<ChangeSet> {
    let report = walk_repo(repo_root)?;
    let mut set = ChangeSet::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for entry in report.entries {
        seen.insert(entry.rel_path.clone());
        let current = file_blake3(&entry.abs_path)?;
        match known.get(&entry.rel_path) {
            None => set.added.push(entry.rel_path),
            Some(prev) if *prev == current => set.unchanged.push(entry.rel_path),
            Some(_) => set.modified.push(entry.rel_path),
        }
    }

    for known_path in known.keys() {
        if !seen.contains(known_path) {
            set.deleted.push(known_path.clone());
        }
    }

    Ok(set)
}
