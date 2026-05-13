//! Workspace / monorepo detection per AS-021 (R39).
//!
//! Precedence (first match wins):
//!   1. Cargo.toml containing a `[workspace]` section
//!   2. pnpm-workspace.yaml
//!   3. nx.json
//!   4. lerna.json
//!   5. multi-manifest heuristic: ≥3 sibling dirs with `package.json` or `go.mod`,
//!      none under the default-exclude list
//!
//! Default-exclude list (applied to heuristic #5 and also to the Foundation-S-004
//! file walker later): `node_modules/`, `vendor/`, `target/`, `dist/`, `build/`,
//! `.venv/`, `testdata/`, `examples/`, `fixtures/`.

use ga_core::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LayoutKind {
    /// Single-module repo — no workspace prefix, no sub-project overhead.
    Flat,
    /// Monorepo detected with the named `marker`.
    Monorepo {
        marker: &'static str,
        /// Sub-project roots relative to `repo_root`. Heuristic-detected
        /// markers populate this; explicit markers (Cargo/pnpm/nx/lerna) leave
        /// it empty — the full members list is parsed by S-004 when walking.
        members: Vec<PathBuf>,
    },
}

/// Default directory names to never recurse into for the multi-manifest
/// heuristic. Keep in lockstep with Foundation-S-004 walker
/// (`ga_parser::walk::EXCLUDED_DIRS`). See that module for the membership
/// rationale; same universal-junk-only policy applies here.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    // VCS metadata
    ".git",
    ".hg",
    ".svn",
    // Node / JS package managers
    "node_modules",
    ".npm",
    ".yarn",
    ".pnp",
    ".pnpm-store",
    // Python
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".hypothesis",
    // Rust
    "target",
    // JVM
    ".gradle",
    ".m2",
    ".ivy2",
    // Elixir
    "_build",
    "deps",
    // Swift / Xcode / CocoaPods
    ".build",
    "DerivedData",
    "Pods",
    // Dart / Flutter
    ".dart_tool",
    // Haskell
    ".stack-work",
    "dist-newstyle",
    ".cabal-sandbox",
    // Generic build outputs
    "dist",
    "build",
    // Framework / monorepo caches
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".astro",
    ".docusaurus",
    ".turbo",
    ".nx",
    ".rush",
    ".vercel",
    ".netlify",
    ".serverless",
    ".cache",
    ".parcel-cache",
    // Coverage / test caches
    "coverage",
    ".nyc_output",
    "htmlcov",
    // IDE / editor metadata
    ".idea",
    ".vscode",
    ".vs",
    ".fleet",
    ".zed",
    ".history",
    // Secret-adjacent (AS-031)
    ".ssh",
    ".aws",
    ".gnupg",
    // PHP / Ruby / Go vendored deps
    "vendor",
];

pub fn detect(repo_root: &Path) -> Result<LayoutKind> {
    if !repo_root.exists() {
        return Err(Error::ConfigCorrupt {
            path: repo_root.display().to_string(),
            reason: "repo_root does not exist".into(),
        });
    }

    // 1. Cargo workspace.
    let cargo = repo_root.join("Cargo.toml");
    if cargo.is_file() {
        let text = std::fs::read_to_string(&cargo).unwrap_or_default();
        if has_cargo_workspace_section(&text) {
            return Ok(LayoutKind::Monorepo {
                marker: "cargo-workspace",
                members: Vec::new(),
            });
        }
    }

    // 2. pnpm.
    if repo_root.join("pnpm-workspace.yaml").is_file() {
        return Ok(LayoutKind::Monorepo {
            marker: "pnpm-workspace",
            members: Vec::new(),
        });
    }

    // 3. nx.
    if repo_root.join("nx.json").is_file() {
        return Ok(LayoutKind::Monorepo {
            marker: "nx",
            members: Vec::new(),
        });
    }

    // 4. lerna.
    if repo_root.join("lerna.json").is_file() {
        return Ok(LayoutKind::Monorepo {
            marker: "lerna",
            members: Vec::new(),
        });
    }

    // 5. Multi-manifest heuristic.
    let members = find_sibling_manifests(repo_root)?;
    if members.len() >= 3 {
        return Ok(LayoutKind::Monorepo {
            marker: "heuristic-multi-manifest",
            members,
        });
    }

    Ok(LayoutKind::Flat)
}

/// Naive check — true if the text contains a TOML `[workspace]` header. We
/// don't need a full TOML parser for this: any occurrence of the line starting
/// with `[workspace]` (with optional whitespace) is sufficient since this is
/// only used for layout detection.
fn has_cargo_workspace_section(text: &str) -> bool {
    text.lines()
        .map(|l| l.trim())
        .any(|l| l == "[workspace]" || l.starts_with("[workspace."))
}

/// Walks at most `max_depth` levels looking for directories (NOT inside
/// DEFAULT_EXCLUDE) that contain a package.json or go.mod. Returns the
/// *directory path* for each hit.
fn find_sibling_manifests(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let mut hits = Vec::new();
    walk(repo_root, repo_root, 0, 3, &mut hits)?;
    Ok(hits)
}

fn walk(
    repo_root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    hits: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > max_depth {
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    // Skip the root itself from the manifest count: a root package.json would
    // indicate a flat repo, which we already allow via the "single-package"
    // test.
    if depth > 0 && (dir.join("package.json").is_file() || dir.join("go.mod").is_file()) {
        let rel = dir.strip_prefix(repo_root).unwrap_or(dir).to_path_buf();
        hits.push(rel);
    }

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if DEFAULT_EXCLUDE.contains(&name.as_str()) {
            continue;
        }
        if name.starts_with('.') && name != "." && name != ".." {
            // hidden dirs like .git, .venv (covered in exclude) — skip just in case.
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(repo_root, &path, depth + 1, max_depth, hits)?;
        }
    }
    Ok(())
}
