//! Shared JSON I/O helpers used by the `ga init` installers (skill,
//! claudemd, permissions, session_hook). Mirrors the atomic-write +
//! read-or-empty pattern from `install/hook/backends.rs` but is exposed
//! as a public utility so the new installer modules don't duplicate it.
//!
//! Kept narrow on purpose — the hook installer already has its own
//! atomic_write_bytes private to backends.rs; we don't refactor that
//! here to avoid touching the v1.5 PR7 hook code paths.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Read a JSON file or return an empty object if the file does not
/// exist / is empty. Errors out on corrupt JSON so the installer never
/// silently overwrites user data.
pub fn read_json_or_empty(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} is corrupt JSON; refusing to overwrite (fix or remove the file)",
            path.display()
        )
    })
}

/// Atomic write via sibling tempfile + persist (POSIX-atomic rename).
pub fn atomic_write_json(path: &Path, value: &Value) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(value).context("serialize JSON")?;
    atomic_write_bytes(path, &serialized)
}

/// Atomic write for arbitrary bytes (markdown, plain text).
pub fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent_dir(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp.")
        .suffix(".ga-init")
        .rand_bytes(8)
        .tempfile_in(parent)
        .with_context(|| format!("create tempfile in {}", parent.display()))?;
    tmp.write_all(bytes).context("write tempfile")?;
    tmp.as_file_mut().sync_all().context("fsync tempfile")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o600));
    }
    tmp.persist(path)
        .map_err(|e| anyhow::anyhow!("rename tempfile -> {}: {}", path.display(), e))?;
    Ok(())
}

/// Read a TOML file or return an empty table if the file does not
/// exist / is empty. Errors out on corrupt TOML.
pub fn read_toml_or_empty(path: &Path) -> Result<toml::value::Table> {
    if !path.exists() {
        return Ok(toml::value::Table::new());
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(toml::value::Table::new());
    }
    let s = std::str::from_utf8(&bytes).context("toml config not utf-8")?;
    toml::from_str(s).with_context(|| {
        format!(
            "{} is corrupt TOML; refusing to overwrite (fix or remove the file)",
            path.display()
        )
    })
}

/// Atomic write of a TOML table.
pub fn atomic_write_toml(path: &Path, value: &toml::value::Table) -> Result<()> {
    let serialized = toml::to_string_pretty(value).context("serialize TOML")?;
    atomic_write_bytes(path, serialized.as_bytes())
}

/// Refuse to write through a symlink at `path` unless the caller
/// explicitly opts in. Defends against attacker-controlled symlinks
/// pointing the installer at sensitive files (e.g. `~/.cursor/mcp.json`
/// → `/etc/passwd`). Mirrors `install/hook/backends.rs::refuse_symlink_unless`.
pub fn refuse_symlink_unless(path: &Path, follow_symlinks: bool) -> Result<()> {
    if follow_symlinks {
        return Ok(());
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow::Error::from(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(anyhow::anyhow!(
            "Refusing to follow symlink at {}; use --follow-symlinks to override.",
            path.display()
        ));
    }
    Ok(())
}

/// Acquire an advisory exclusive flock on a sidecar lockfile next to
/// `path`. Returns a guard that holds the lock for its lifetime — drop
/// it to release. Used to serialize cross-process `ga init` runs that
/// mutate the same JSON file (e.g. .claude/settings.json gets touched
/// by permissions + session_hook + reindex hook installers).
///
/// Fail-soft: if lock acquisition fails (filesystem doesn't support
/// flock, e.g. NFS without proper config), the guard is still returned
/// with the file handle but without a lock. Callers proceed best-effort.
pub fn lock_file(path: &Path) -> Result<FileLockGuard> {
    ensure_parent_dir(path)?;
    let lock_path = lock_path_for(path);
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open lockfile {}", lock_path.display()))?;
    // Block until acquired. ga init is interactive; waiting is fine.
    // Use fully-qualified trait call to silence Rust 1.95's MSRV lint
    // about `File::lock` (stable 1.89) vs GA's MSRV 1.85. fs4's trait
    // wraps the same advisory flock syscall on POSIX. Fail-soft: if
    // the filesystem doesn't support advisory locks, the operation
    // returns an error which we ignore — best-effort serialization.
    let _ = <std::fs::File as fs4::fs_std::FileExt>::lock_exclusive(&file);
    Ok(FileLockGuard { file })
}

pub struct FileLockGuard {
    file: std::fs::File,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = <std::fs::File as fs4::fs_std::FileExt>::unlock(&self.file);
    }
}

fn lock_path_for(target: &Path) -> std::path::PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".ga-lock");
    std::path::PathBuf::from(s)
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
    }
    Ok(())
}
