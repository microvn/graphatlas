//! Cluster C6 — AS-012 `affected_configs` discovery.
//!
//! Recursively walks the repo filesystem under `Store::metadata().repo_root`,
//! scanning `.env*`, `*.yaml`/`.yml`, `*.toml`, `*.json` files for
//! case-sensitive mentions of the seed symbol or any seed file stem. Emits
//! one [`AffectedConfig`] per `(path, line)` hit.
//!
//! Heavy / vendored directories are skipped: `.git`, `node_modules`, `target`,
//! `vendor`, `dist`, `build`, `.graphatlas`. CI vendor dirs (`.github`,
//! `.gitlab`, `.gitea`, `.circleci`), tool-cache dirs from other graph tools
//! (`.arbor`, `.playwright-mcp`, `.codegraph`, `.code-review-graph`, `.axon`,
//! `.gitnexus`, `.gstack`), and IDE workspaces (`.idea`, `.vscode`) are also
//! skipped — they hold repo plumbing, not app runtime config. CORE-1
//! (2026-05-22). Well-known repo-root tooling files (`.golangci.yml`,
//! `.eslintrc.json`, `.travis.yml`, etc.) are filtered by filename via
//! [`SKIP_FILES`] so app yaml/toml/json continues to surface.

use super::types::AffectedConfig;
use crate::common;
use ga_core::Result;
use ga_index::Store;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    // Heavy / vendored
    ".git",
    ".graphatlas",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    // CI vendor dirs — CORE-1 (2026-05-22). These hold pipeline metadata, not
    // app runtime config; mentioning a symbol there is repo plumbing, not a
    // runtime dependency. Confirmed leak on gin (.github/) and tokio
    // (.github/labeler.yml).
    ".github",
    ".gitlab",
    ".gitea",
    ".circleci",
    // Tool-cache / artifact dirs from other graph + code-review tools.
    // These end up inside user repos when users try alternatives — treat as
    // user-installed caches, not config. Confirmed leak on cardshield.
    ".arbor",
    ".playwright-mcp",
    ".codegraph",
    ".code-review-graph",
    ".axon",
    ".gitnexus",
    ".gstack",
    // IDE workspace settings — editor metadata, not runtime config.
    ".idea",
    ".vscode",
];

/// Repo-root tooling/lint configs that match the .yaml/.toml/.json extension
/// gate but are *not* app runtime config. Exact filename match keeps the list
/// conservative — a yaml *not* on this list will still surface (so app yaml
/// like `config/prod.yaml` is unaffected). CORE-1 (2026-05-22).
const SKIP_FILES: &[&str] = &[
    // Go ecosystem tooling
    ".golangci.yml",
    ".golangci.yaml",
    ".goreleaser.yml",
    ".goreleaser.yaml",
    // JS / TS ecosystem linters / formatters
    ".eslintrc.json",
    ".eslintrc.yaml",
    ".eslintrc.yml",
    ".prettierrc.json",
    ".prettierrc.yaml",
    ".prettierrc.yml",
    ".stylelintrc.json",
    ".stylelintrc.yaml",
    ".stylelintrc.yml",
    ".babelrc.json",
    ".markdownlintrc.json",
    ".markdownlint.json",
    ".markdownlint.yaml",
    ".markdownlint.yml",
    // CI files at repo root (no parent dir to skip)
    ".travis.yml",
    ".travis.yaml",
    "dependabot.yml",
    "dependabot.yaml",
    "renovate.json",
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
    // CORE-1: skip well-known tooling configs by exact filename even though
    // they match the extension gate. Keeps app config (config/*.yaml,
    // pyproject.toml, package.json) flowing through.
    if SKIP_FILES.contains(&name) {
        return false;
    }
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
