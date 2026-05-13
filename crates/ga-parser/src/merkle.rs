//! Bounded Merkle root hash — S-005 AS-013.
//!
//! Input ingredients:
//!   1. Up to `bound_n` directories found at depth ≤ `depth_cap` within
//!      `repo_root`, each contributing `(relative_path, mtime_ns)`.
//!   2. `.git/index` mtime_ns if the file exists.
//!   3. `.git/HEAD` full content bytes if the file exists.
//!
//! All inputs go through BLAKE3 in a stable lexicographic order so the hash
//! is deterministic across filesystems with different directory-enumeration
//! ordering.

use ga_core::{Error, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct MerkleConfig {
    /// Cap on directories fed into the hash. Protects against monorepos with
    /// thousands of subdirs (AS-013 "bounded N≤32 dirs").
    pub bound_n: usize,
    /// Max relative depth walked when enumerating directories. Spec says
    /// "depth ≤ 2" — depth=0 is repo_root itself, depth=1/2 are children.
    pub depth_cap: u32,
}

impl Default for MerkleConfig {
    fn default() -> Self {
        Self {
            bound_n: 32,
            depth_cap: 2,
        }
    }
}

/// BLAKE3-hash the bounded repo signature. Returns raw 32-byte digest.
pub fn compute_root_hash(repo_root: &Path, cfg: &MerkleConfig) -> Result<[u8; 32]> {
    if !repo_root.exists() {
        return Err(Error::ConfigCorrupt {
            path: repo_root.display().to_string(),
            reason: "repo_root does not exist".into(),
        });
    }

    let mut dirs: Vec<(PathBuf, u128)> = Vec::new();
    collect_dirs(repo_root, repo_root, 0, cfg.depth_cap, &mut dirs);

    // Stable sort by relative path to normalize across FS-orderings.
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    if dirs.len() > cfg.bound_n {
        dirs.truncate(cfg.bound_n);
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"ga-merkle-v1\n");
    for (rel, mtime_ns) in &dirs {
        hasher.update(b"D:");
        hasher.update(rel.as_os_str().as_encoded_bytes());
        hasher.update(b":");
        hasher.update(&mtime_ns.to_le_bytes());
        hasher.update(b"\n");
    }

    // .git/index mtime_ns (optional).
    let git_index = repo_root.join(".git/index");
    if let Ok(meta) = std::fs::metadata(&git_index) {
        if let Some(m) = mtime_ns_of(&meta) {
            hasher.update(b"GI:");
            hasher.update(&m.to_le_bytes());
            hasher.update(b"\n");
        }
    }

    // .git/HEAD full content (optional).
    let git_head = repo_root.join(".git/HEAD");
    if let Ok(bytes) = std::fs::read(&git_head) {
        hasher.update(b"GH:");
        hasher.update(&bytes);
        hasher.update(b"\n");
    }

    let out = hasher.finalize();
    Ok(*out.as_bytes())
}

fn collect_dirs(
    repo_root: &Path,
    dir: &Path,
    depth: u32,
    depth_cap: u32,
    out: &mut Vec<(PathBuf, u128)>,
) {
    if depth > depth_cap {
        return;
    }

    // Record this dir's mtime except repo_root itself (depth 0 — redundant
    // with the child entries and noisy since it changes on any child write).
    if depth > 0 {
        if let Ok(meta) = std::fs::metadata(dir) {
            if let Some(mtime) = mtime_ns_of(&meta) {
                let rel = dir.strip_prefix(repo_root).unwrap_or(dir).to_path_buf();
                out.push((rel, mtime));
            }
        }
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        // Skip hidden-name dirs except .git (which we handle explicitly) and
        // the EXCLUDED_DIRS used by the walker so the hash doesn't pick up
        // build-tool noise.
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if crate::walk::is_excluded_dir(&name) {
            continue;
        }
        // Walk only real directories; symlinks are ignored for hashing (their
        // target tree is hashed if they stay inside the repo but the walker
        // path does that check and we keep the Merkle input simple).
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            collect_dirs(repo_root, &entry.path(), depth + 1, depth_cap, out);
        }
    }
}

fn mtime_ns_of(meta: &std::fs::Metadata) -> Option<u128> {
    let mtime = meta.modified().ok()?;
    mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}
