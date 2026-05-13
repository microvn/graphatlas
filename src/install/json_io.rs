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
