//! `graphatlas list` — enumerate per-repo caches under `~/.graphatlas/`.
//! Foundation-C12: each cache dir is `<repo-name>-<6-hex-sha256>`; reading
//! `metadata.json.repo_root` gives the full repo path for reverse lookup.

use crate::metadata::Metadata;
use ga_core::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub dir_name: String,
    pub dir: PathBuf,
    pub repo_root: String,
    pub size_bytes: u64,
    pub last_indexed_unix: u64,
    pub index_state: ga_core::IndexState,
}

/// Scan `cache_root` (typically `~/.graphatlas`) and return one entry per valid
/// cache. Silently skips dirs with missing or corrupt metadata.json — callers
/// (`graphatlas list`, `graphatlas doctor`) handle presentation.
pub fn list_caches(cache_root: &Path) -> Result<Vec<CacheEntry>> {
    if !cache_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(cache_root) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let metadata_path = path.join("metadata.json");
        if !metadata_path.is_file() {
            continue;
        }
        let bytes = match std::fs::read(&metadata_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let md: Metadata = match serde_json::from_slice(&bytes) {
            Ok(m) => m,
            Err(_) => continue, // Corrupt metadata — skip; doctor reports.
        };
        let dir_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let size_bytes = dir_size(&path);
        let last_indexed_unix = md.committed_at.unwrap_or(md.indexed_at);
        out.push(CacheEntry {
            dir_name,
            dir: path,
            repo_root: md.repo_root,
            size_bytes,
            last_indexed_unix,
            index_state: md.index_state,
        });
    }
    Ok(out)
}

fn dir_size(dir: &Path) -> u64 {
    fn walk(p: &Path, acc: &mut u64) {
        let entries = match std::fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_file() => {
                    if let Ok(m) = entry.metadata() {
                        *acc += m.len();
                    }
                }
                Ok(ft) if ft.is_dir() => walk(&path, acc),
                _ => {}
            }
        }
    }
    let mut total = 0u64;
    walk(dir, &mut total);
    total
}

/// Format bytes as human-readable (KB/MB/GB). Used by CLI presenter.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{}MB", bytes / MB)
    } else if bytes >= KB {
        format!("{}KB", bytes / KB)
    } else {
        format!("{bytes}B")
    }
}

/// Format unix timestamp as "Nd ago" / "Nh ago" / "just now".
pub fn format_age(last_indexed_unix: u64, now_unix: u64) -> String {
    if last_indexed_unix == 0 || last_indexed_unix > now_unix {
        return "-".to_string();
    }
    let delta = now_unix - last_indexed_unix;
    match delta {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", delta / 60),
        3600..=86_399 => format!("{}h ago", delta / 3600),
        _ => format!("{}d ago", delta / 86_400),
    }
}
