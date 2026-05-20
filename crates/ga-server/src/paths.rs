//! Path safety helpers — Spec A A-C10 + AS-018 / AS-019 invariants.
//!
//! Defense layers (must ALL pass before a path reaches subprocess spawn):
//!  1. Canonicalize. Rejects nonexistent paths up-front (AS-016).
//!  2. Reject `..` components in the *input* (canonicalize already
//!     resolves them, but we reject early to give a clearer error).
//!  3. Reject paths that resolve into `cache_root` (AS-018).
//!  4. Reject if the canonical path's ancestry contains an external
//!     symlink — i.e. a symlink whose target sits outside the input
//!     repo root (AS-019). This catches `repo/secrets -> /etc`.
//!  5. Reject if the canonical resolves to a file (AS-017).

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRejection {
    NotFound,
    NotDirectory,
    Unsafe(&'static str),
    ExternalSymlink(String),
}

impl PathRejection {
    pub fn code(&self) -> &'static str {
        match self {
            PathRejection::NotFound => "path_not_found",
            PathRejection::NotDirectory => "path_not_directory",
            PathRejection::Unsafe(_) => "path_unsafe",
            PathRejection::ExternalSymlink(_) => "path_contains_external_symlink",
        }
    }

    pub fn message(&self) -> String {
        match self {
            PathRejection::NotFound => "path does not exist".into(),
            PathRejection::NotDirectory => "path is not a directory".into(),
            PathRejection::Unsafe(why) => (*why).into(),
            PathRejection::ExternalSymlink(s) => {
                format!("path contains symlink pointing outside repo: {}", s)
            }
        }
    }
}

/// Validate `input` against the rules above. Returns the canonical
/// directory on success.
///
/// `cache_root` is the canonical `~/.graphatlas` dir; we reject inputs
/// that resolve into it (path traversal would let an attacker tell us
/// to "index" the cache directory itself).
pub fn validate_repo_path(input: &Path, cache_root: &Path) -> Result<PathBuf, PathRejection> {
    // Layer 2 — reject ".." in raw input before canonicalize swallows it.
    for c in input.components() {
        if matches!(c, Component::ParentDir) {
            return Err(PathRejection::Unsafe("path contains '..' component"));
        }
    }

    // Layer 1 — canonicalize (resolves symlinks fully).
    let canonical = input.canonicalize().map_err(|_| PathRejection::NotFound)?;

    // Layer 5 — must be a directory.
    let meta = std::fs::metadata(&canonical).map_err(|_| PathRejection::NotFound)?;
    if !meta.is_dir() {
        return Err(PathRejection::NotDirectory);
    }

    // Layer 3 — reject if resolves into cache_root. Use case-folded
    // comparison on macOS APFS default (case-insensitive). Strict
    // prefix check on canonical absolute paths — not substring.
    let cache_canonical = cache_root
        .canonicalize()
        .unwrap_or_else(|_| cache_root.to_path_buf());
    if path_starts_with_case_insensitive(&canonical, &cache_canonical) {
        return Err(PathRejection::Unsafe("path resolves into cache directory"));
    }

    // Layer 4 — walk children one level looking for symlinks that point
    // outside the (canonical) repo root. Phase 1 only checks immediate
    // children to keep the cost bounded; the reindex walker is invoked
    // with `follow_symlinks=false` per A-C10 invariant for deeper
    // defense. (See README of ga-parser when wiring S-005 spawn.)
    if let Ok(entries) = std::fs::read_dir(&canonical) {
        for entry in entries.flatten() {
            let p = entry.path();
            let symlink_meta = match entry.file_type() {
                Ok(ft) if ft.is_symlink() => p.clone(),
                _ => continue,
            };
            // Resolve the link target and check ancestry.
            if let Ok(target) = std::fs::read_link(&symlink_meta) {
                let resolved = if target.is_absolute() {
                    target.canonicalize().unwrap_or(target)
                } else {
                    canonical
                        .join(target)
                        .canonicalize()
                        .unwrap_or_else(|_| canonical.join("__unresolved__"))
                };
                if !resolved.starts_with(&canonical) {
                    return Err(PathRejection::ExternalSymlink(
                        symlink_meta.display().to_string(),
                    ));
                }
            }
        }
    }

    Ok(canonical)
}

fn path_starts_with_case_insensitive(needle: &Path, anchor: &Path) -> bool {
    // On case-sensitive FS the default `starts_with` is fine, but APFS
    // default + Windows are case-insensitive. Compare lossy lowercase
    // strings to be safe; this is only used for the cache_root guard,
    // not for security-sensitive auth.
    let n = needle.to_string_lossy().to_lowercase();
    let a = anchor.to_string_lossy().to_lowercase();
    let n = n.trim_end_matches('/').trim_end_matches('\\');
    let a = a.trim_end_matches('/').trim_end_matches('\\');
    n == a || n.starts_with(&format!("{}/", a)) || n.starts_with(&format!("{}\\", a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reject_dotdot_components() {
        let cache = tempdir().unwrap();
        let probe = PathBuf::from("/tmp/../etc");
        let err = validate_repo_path(&probe, cache.path()).unwrap_err();
        assert!(matches!(err, PathRejection::Unsafe(_)));
    }

    #[test]
    fn reject_nonexistent_path() {
        let cache = tempdir().unwrap();
        let err = validate_repo_path(Path::new("/tmp/__nope__/__xyz__"), cache.path()).unwrap_err();
        assert_eq!(err, PathRejection::NotFound);
    }

    #[test]
    fn reject_file_path() {
        let cache = tempdir().unwrap();
        let f = tempdir().unwrap();
        let file = f.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let err = validate_repo_path(&file, cache.path()).unwrap_err();
        assert_eq!(err, PathRejection::NotDirectory);
    }

    #[test]
    fn reject_path_into_cache_root() {
        let cache = tempdir().unwrap();
        let inside = cache.path().join("victim-cache");
        std::fs::create_dir(&inside).unwrap();
        let err = validate_repo_path(&inside, cache.path()).unwrap_err();
        assert!(matches!(err, PathRejection::Unsafe(_)));
    }

    #[test]
    fn accept_normal_dir() {
        let cache = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let ok = validate_repo_path(repo.path(), cache.path()).unwrap();
        assert!(ok.is_dir());
    }

    #[test]
    #[cfg(unix)]
    fn reject_external_symlink_in_repo() {
        let cache = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let outside = tempdir().unwrap(); // Pretend /etc.
                                          // Need to leak outside path so the link target survives the
                                          // test — actually `tempdir()` keeps it alive via guard, fine.
        std::os::unix::fs::symlink(outside.path(), repo.path().join("evil")).unwrap();
        let err = validate_repo_path(repo.path(), cache.path()).unwrap_err();
        assert!(matches!(err, PathRejection::ExternalSymlink(_)));
    }

    #[test]
    #[cfg(unix)]
    fn accept_internal_symlink() {
        let cache = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let target = repo.path().join("inner");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, repo.path().join("link")).unwrap();
        assert!(validate_repo_path(repo.path(), cache.path()).is_ok());
    }
}
