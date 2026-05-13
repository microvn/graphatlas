//! Cluster C6 — AS-012 `affected_configs` discovery.
//!
//! Recursively walks the repo filesystem under `Store::metadata().repo_root`,
//! scanning `.env*`, `*.yaml`/`.yml`, `*.toml`, `*.json` files for
//! case-sensitive mentions of the seed symbol or any seed file stem. Emits
//! one [`AffectedConfig`] per `(path, line)` hit.
//!
//! Heavy / vendored directories are skipped: `.git`, `node_modules`, `target`,
//! `vendor`, `dist`, `build`, `.graphatlas`. This keeps the scan bounded on
//! monorepos without pulling in a gitignore parser for v1.

use super::types::AffectedConfig;
use crate::common;
use ga_core::Result;
use ga_index::Store;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".graphatlas",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
];

pub(super) fn collect_affected_configs(
    store: &Store,
    seed_symbol: &str,
    seed_stems: &[String],
) -> Result<Vec<AffectedConfig>> {
    if !common::is_safe_ident(seed_symbol) {
        return Ok(Vec::new());
    }

    let repo_root = store.metadata().repo_root.clone();
    if repo_root.is_empty() {
        return Ok(Vec::new());
    }
    let repo_root = PathBuf::from(repo_root);

    let mut out: Vec<AffectedConfig> = Vec::new();
    walk(&repo_root, &repo_root, seed_symbol, seed_stems, &mut out);

    out.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    Ok(out)
}

/// Recursive walker. `base` = absolute repo root used to compute relative
/// paths; `dir` = current working directory.
fn walk(base: &Path, dir: &Path, symbol: &str, stems: &[String], out: &mut Vec<AffectedConfig>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk(base, &path, symbol, stems, out);
        } else if ft.is_file() && is_config_file(name) {
            scan_file(base, &path, symbol, stems, out);
        }
    }
}

fn is_config_file(name: &str) -> bool {
    if name.starts_with(".env") {
        return true;
    }
    matches!(
        name.rsplit('.').next(),
        Some("yaml") | Some("yml") | Some("toml") | Some("json")
    ) && {
        // Require a dot — names like "yaml" without extension don't qualify.
        name.contains('.')
    }
}

fn scan_file(
    base: &Path,
    abs_path: &Path,
    symbol: &str,
    stems: &[String],
    out: &mut Vec<AffectedConfig>,
) {
    let Ok(bytes) = std::fs::read(abs_path) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return;
    };
    let Ok(rel) = abs_path.strip_prefix(base) else {
        return;
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");

    for (i, line) in text.lines().enumerate() {
        if line_mentions(line, symbol, stems) {
            out.push(AffectedConfig {
                path: rel_str.clone(),
                line: (i + 1) as u32,
            });
        }
    }
}

fn line_mentions(line: &str, symbol: &str, stems: &[String]) -> bool {
    if line.contains(symbol) {
        return true;
    }
    stems
        .iter()
        .any(|stem| !stem.is_empty() && line.contains(stem))
}
