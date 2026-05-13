//! Cache layout + Unix permission enforcement.
//! Foundation-C12: per-repo cache at `~/.graphatlas/<repo-name>-<6-hex-sha256>/`.
//! Foundation-C8: dir mode 0700, file mode 0600, refuse-open otherwise.

use ga_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 6-hex-digit prefix of sha256(repo_root) — per Foundation-C12.
pub fn short_hash(repo_root: &str) -> String {
    let trimmed = repo_root.trim_end_matches('/');
    let canonical = if trimmed.is_empty() { "/" } else { trimmed };
    let hash = Sha256::digest(canonical.as_bytes());
    hex_prefix(&hash, 6)
}

fn hex_prefix(bytes: &[u8], nchars: usize) -> String {
    let mut s = String::with_capacity(nchars);
    for b in bytes {
        if s.len() >= nchars {
            break;
        }
        s.push_str(&format!("{b:02x}"));
    }
    s.truncate(nchars);
    s
}

/// Resolved paths for one repo's cache.
#[derive(Clone)]
pub struct CacheLayout {
    dir: PathBuf,
    repo_name: String,
}

impl CacheLayout {
    /// Compute layout for `repo_root` under `cache_root` (typically `~/.graphatlas`).
    pub fn for_repo(cache_root: &Path, repo_root: &Path) -> Self {
        let repo_root_str = repo_root.to_string_lossy().to_string();
        let repo_name = extract_repo_name(&repo_root_str);
        let hash = short_hash(&repo_root_str);
        let dir_name = format!("{repo_name}-{hash}");
        let dir = cache_root.join(&dir_name);
        Self { dir, repo_name }
    }

    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn dir_name(&self) -> String {
        self.dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn graph_db(&self) -> PathBuf {
        self.dir.join("graph.db")
    }

    pub fn metadata_json(&self) -> PathBuf {
        self.dir.join("metadata.json")
    }

    pub fn lock_pid(&self) -> PathBuf {
        self.dir.join("lock.pid")
    }

    /// Ensure the cache directory exists with mode 0700. Refuse open if it already
    /// exists with more permissive mode (AS-029 error path).
    pub fn ensure_dir(&self) -> Result<()> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir)?;
            set_mode_0700(&self.dir)?;
            return Ok(());
        }
        verify_dir_perms(&self.dir)?;
        Ok(())
    }
}

fn extract_repo_name(repo_root: &str) -> String {
    let trimmed = repo_root.trim_end_matches('/');
    if trimmed.is_empty() {
        return "root".to_string();
    }
    trimmed
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("root")
        .to_string()
}

#[cfg(unix)]
fn set_mode_0700(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode_0700(_path: &Path) -> Result<()> {
    // Windows: ACL work deferred to v1.1 per PLAN R32.
    Ok(())
}

/// AS-029 error path: refuse open if dir mode > 0700.
#[cfg(unix)]
pub fn verify_dir_perms(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(dir)?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o700 {
        return Err(Error::ConfigCorrupt {
            path: dir.display().to_string(),
            reason: format!(
                "cache directory has unsafe permissions (0{mode:o}); run `chmod 0700 {}` or remove cache",
                dir.display()
            ),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn verify_dir_perms(_dir: &Path) -> Result<()> {
    Ok(())
}

/// AS-029 error path: refuse open if file mode > 0600.
#[cfg(unix)]
pub fn verify_file_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(Error::ConfigCorrupt {
            path: path.display().to_string(),
            reason: format!(
                "cache file has unsafe permissions (0{mode:o}); run `chmod 0600 {}` or remove cache",
                path.display()
            ),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn verify_file_perms(_path: &Path) -> Result<()> {
    Ok(())
}

/// Write `contents` to `path` with mode 0600 on Unix. Atomic via rename-after-write.
pub fn write_file_0600(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::ConfigCorrupt {
        path: path.display().to_string(),
        reason: "no parent directory".into(),
    })?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("out")
    ));
    std::fs::write(&tmp, contents)?;
    set_mode_0600(&tmp)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_mode_0600(path: &Path) -> Result<()> {
    chmod_0600(path)
}

#[cfg(not(unix))]
fn set_mode_0600(_path: &Path) -> Result<()> {
    Ok(())
}

/// Publicly clamp a file to mode 0600. On non-Unix this is a no-op (Windows
/// ACL work deferred to v1.1 per Foundation-C8 Windows row).
#[cfg(unix)]
pub fn chmod_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn chmod_0600(_path: &Path) -> Result<()> {
    Ok(())
}

/// Create (if missing) and clamp cache root to mode 0700. Used for
/// `~/.graphatlas` (default) or any `GRAPHATLAS_CACHE_DIR` override.
pub fn ensure_cache_root(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Foundation-C8: validate a user-provided cache root (typically from
/// `GRAPHATLAS_CACHE_DIR`). Reject paths under sensitive dirs or with
/// too-permissive modes. Nonexistent paths are accepted (we'll create
/// with 0700 later).
pub fn validate_cache_dir_override(root: &Path) -> Result<()> {
    let as_str = root.to_string_lossy();

    // Secret-shaped user dirs — reject any path inside these (anchored to either
    // start-of-string or a `/` boundary so `.ssh-backup` doesn't trigger).
    let has_segment = |needle: &str| {
        as_str == needle
            || as_str.starts_with(&format!("{needle}/"))
            || as_str.contains(&format!("/{}/", needle.trim_start_matches('/')))
            || as_str.ends_with(&format!("/{}", needle.trim_start_matches('/')))
    };
    const SEGMENTS: &[&str] = &[".ssh", ".gnupg"];
    for seg in SEGMENTS {
        if has_segment(seg) {
            return Err(Error::ConfigCorrupt {
                path: root.display().to_string(),
                reason: format!(
                    "GRAPHATLAS_CACHE_DIR points to an unsafe location (contains `{seg}` segment). \
                     Refuse to place cache there."
                ),
            });
        }
    }
    if as_str.contains("/.config/gh") {
        return Err(Error::ConfigCorrupt {
            path: root.display().to_string(),
            reason: "GRAPHATLAS_CACHE_DIR points to an unsafe location (inside .config/gh). \
                     Refuse to place cache there."
                .into(),
        });
    }

    // System dirs /etc and /var subtrees. We intentionally carve out /var/tmp
    // and /var/folders (macOS per-user temp) since the spec's blanket "/var"
    // rule would otherwise block the platform-default $TMPDIR on macOS.
    let forbidden_system_prefix = |p: &str| {
        if p == "/etc" || p.starts_with("/etc/") {
            return Some("/etc");
        }
        if p == "/var" {
            return Some("/var");
        }
        if p.starts_with("/var/") {
            // Explicit allow-list for per-user/ephemeral tmpdirs.
            if p.starts_with("/var/tmp/") || p == "/var/tmp" {
                return None;
            }
            if p.starts_with("/var/folders/") || p == "/var/folders" {
                return None;
            }
            return Some("/var");
        }
        None
    };
    if let Some(tok) = forbidden_system_prefix(as_str.as_ref()) {
        return Err(Error::ConfigCorrupt {
            path: root.display().to_string(),
            reason: format!(
                "GRAPHATLAS_CACHE_DIR points to an unsafe system location (`{tok}`). \
                 Refuse to place cache there."
            ),
        });
    }

    // If the dir already exists, require mode <= 0700 (matches dir-perms check).
    if root.exists() {
        validate_existing_dir(root)?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_existing_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(dir)?;
    let mode = meta.permissions().mode() & 0o777;
    if mode > 0o700 {
        return Err(Error::ConfigCorrupt {
            path: dir.display().to_string(),
            reason: format!(
                "GRAPHATLAS_CACHE_DIR target has unsafe mode 0{mode:o} (must be <= 0700). \
                 Fix with `chmod 0700 {}` or pick a different path.",
                dir.display()
            ),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_existing_dir(_dir: &Path) -> Result<()> {
    // Windows ACL validation deferred to v1.1 per Foundation-C8.
    Ok(())
}
