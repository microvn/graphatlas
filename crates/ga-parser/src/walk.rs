//! Indexer walk (AS-031). Hardened per R34 + R35:
//!   - `symlink_metadata` to detect symlinks without following them
//!   - canonicalize target; if it escapes `repo_root` → skip + warn
//!   - default-exclude secret-shaped paths (`.env*`, `id_rsa*`, `*.pem`, `*.key`)
//!   - default-exclude standard noise dirs (shared with monorepo detector)
//!
//! Defends the "prompt-injection-via-indexed-content" attack chain flagged in
//! Foundation-C13 + `docs/THREAT_MODEL.md`.

use ga_core::{Error, Lang, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

/// Dirs never recursed into. Keep in lockstep with `ga_index::monorepo::DEFAULT_EXCLUDE`.
///
/// Membership rule: only universal junk — package-manager caches, build
/// artefacts, IDE metadata, and secret-adjacent paths. Project-specific
/// junk (e.g. `.graphatlas-bench-cache/`) belongs in `.gitignore`, which
/// the walker honours separately via the `ignore` crate.
///
/// Conspicuously absent: `examples`, `fixtures`, `testdata`. These contain
/// real source code in the wider ecosystem (Rust `examples/*.rs` are
/// build targets; Python `fixtures/*.py` import internal APIs and
/// generate caller edges). Skipping them by default produced false
/// negatives in code-intel queries. Projects that genuinely want them
/// excluded should `.gitignore` them.
///
/// `.ssh` / `.aws` / `.gnupg` stay here per AS-031: entire subtrees are
/// secret-adjacent regardless of any user configuration.
pub const EXCLUDED_DIRS: &[&str] = &[
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
    // JVM (Gradle / Maven / Ivy)
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

#[derive(Debug)]
pub struct WalkEntry {
    pub rel_path: PathBuf,
    pub abs_path: PathBuf,
    pub lang: Lang,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct WalkReport {
    pub entries: Vec<WalkEntry>,
    /// Paths whose canonical form escapes `repo_root`. Surfaced so doctor can
    /// log a warning per AS-031.
    pub skipped_symlinks: Vec<PathBuf>,
    /// Paths that matched the secret-shape rules.
    pub skipped_secrets: Vec<PathBuf>,
}

pub fn walk_repo(repo_root: &Path) -> Result<WalkReport> {
    if !repo_root.exists() {
        return Err(Error::ConfigCorrupt {
            path: repo_root.display().to_string(),
            reason: "repo_root does not exist".into(),
        });
    }
    let canonical_root = repo_root.canonicalize().map_err(|e| Error::ConfigCorrupt {
        path: repo_root.display().to_string(),
        reason: format!("canonicalize failed: {e}"),
    })?;

    let mut report = WalkReport::default();
    let gi = build_root_gitignore(&canonical_root);
    walk_dir(&canonical_root, &canonical_root, &gi, &mut report)?;
    Ok(report)
}

/// Build a gitignore matcher rooted at `root`. Reads the standard set of
/// ignore files at repo root: `.gitignore`, `.ignore` (ripgrep/fd
/// convention for tool-specific ignores), and `.git/info/exclude`. Any
/// I/O failure is non-fatal — the matcher silently degrades to an empty
/// ruleset and `EXCLUDED_DIRS` continues to act as the safety net.
///
/// Limitation: nested per-directory `.gitignore` files are NOT stacked.
/// Most repos place all rules at the root; sub-directory ignores are
/// rare. Full hierarchical support would require switching to
/// `ignore::WalkBuilder`, which conflicts with the AS-031 symlink
/// guarantees in this hand-rolled walker.
fn build_root_gitignore(root: &Path) -> Gitignore {
    let mut b = GitignoreBuilder::new(root);
    let _ = b.add(root.join(".gitignore"));
    let _ = b.add(root.join(".ignore"));
    let _ = b.add(root.join(".git").join("info").join("exclude"));
    b.build().unwrap_or_else(|_| Gitignore::empty())
}

fn walk_dir(root: &Path, dir: &Path, gi: &Gitignore, report: &mut WalkReport) -> Result<()> {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(e) => e.flatten().collect(),
        Err(_) => return Ok(()), // permission denied / concurrent remove — silent skip
    };
    // Sort for deterministic traversal order across OS/filesystem implementations.
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let file_name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue, // non-UTF8 filename — skip
        };

        // symlink_metadata — do NOT follow symlinks automatically. If this is
        // a symlink, we compare its canonical target against root before
        // recursing/indexing.
        let sym_meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if sym_meta.file_type().is_symlink() && !symlink_inside_root(&path, root) {
            let rel = relativize(&path, root);
            let target = std::fs::read_link(&path).unwrap_or_else(|_| path.clone());
            crate::logs::emit_symlink_escape(&rel, &target);
            report.skipped_symlinks.push(rel);
            continue;
        }

        if sym_meta.file_type().is_dir() || resolves_to_dir(&path) {
            if EXCLUDED_DIRS.contains(&file_name.as_str()) {
                continue;
            }
            // For symlinks staying inside the repo, also check the canonical
            // target's path segments against EXCLUDED_DIRS. Prevents a link
            // named "shortcut" from smuggling us into e.g. target/debug/.
            if sym_meta.file_type().is_symlink() && canonical_has_excluded_segment(&path) {
                continue;
            }
            // Honour `.gitignore` for project-specific exclusions
            // (e.g. `.graphatlas-bench-cache/`, `.code-review-graph/`).
            // EXCLUDED_DIRS already short-circuited universal junk above;
            // gitignore is the long tail that varies per repo.
            if gi.matched_path_or_any_parents(&path, true).is_ignore() {
                continue;
            }
            walk_dir(root, &path, gi, report)?;
            continue;
        }

        // File-level gitignore check. Same rationale as the directory
        // branch above — let projects exclude individual generated files
        // (e.g. `*.generated.ts`) without us hardcoding patterns.
        if gi.matched_path_or_any_parents(&path, false).is_ignore() {
            continue;
        }

        if is_secret_shaped(&file_name) {
            report.skipped_secrets.push(relativize(&path, root));
            continue;
        }

        // AS-031 extra rule: files at mode 0600 with no recognized source
        // extension are tracked as probable secrets. Distinct from the plain
        // "no source extension" skip below because `doctor` surfaces the
        // `skipped_secrets` list to operators.
        let source_lang = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(Lang::from_ext);
        #[cfg(unix)]
        if source_lang.is_none() && is_mode_0600(&sym_meta) {
            report.skipped_secrets.push(relativize(&path, root));
            continue;
        }

        // Source file? Pull language from extension.
        let lang = match source_lang {
            Some(l) => l,
            None => continue,
        };
        let size = sym_meta.len();
        let rel_path = relativize(&path, root);
        report.entries.push(WalkEntry {
            rel_path,
            abs_path: path,
            lang,
            size,
        });
    }

    Ok(())
}

/// Return true if the canonical path contains any segment listed in
/// [`EXCLUDED_DIRS`]. Used when resolving a symlink to catch smuggling into
/// excluded subtrees.
fn canonical_has_excluded_segment(link: &Path) -> bool {
    let Ok(target) = link.canonicalize() else {
        return true; // dangling → conservative skip
    };
    target.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| EXCLUDED_DIRS.contains(&s))
            .unwrap_or(false)
    })
}

fn symlink_inside_root(link: &Path, root: &Path) -> bool {
    // Canonicalize the symlink target. If it can't be canonicalized (dangling
    // symlink) treat as unsafe and skip.
    match link.canonicalize() {
        Ok(target) => target.starts_with(root),
        Err(_) => false,
    }
}

fn resolves_to_dir(path: &Path) -> bool {
    // For symlinks, check the target is a directory (after we've confirmed
    // it stays inside root).
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

pub fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

fn relativize(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

#[cfg(unix)]
fn is_mode_0600(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    (meta.permissions().mode() & 0o777) == 0o600
}

/// Secret-shape matcher per R35.
fn is_secret_shaped(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    // .env, .env.local, .env.production, ...
    if lower == ".env" || lower.starts_with(".env.") {
        return true;
    }
    // id_rsa, id_rsa.pub, id_ed25519, ...
    if lower == "id_rsa"
        || lower.starts_with("id_rsa.")
        || lower == "id_ed25519"
        || lower.starts_with("id_ed25519.")
        || lower == "id_dsa"
        || lower.starts_with("id_dsa.")
    {
        return true;
    }
    // *.pem, *.key, *.crt — common PKI extensions.
    if lower.ends_with(".pem") || lower.ends_with(".key") || lower.ends_with(".crt") {
        return true;
    }
    // .pgpass, .netrc — classic credential files.
    if lower == ".pgpass" || lower == ".netrc" {
        return true;
    }
    false
}
