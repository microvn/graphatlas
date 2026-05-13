//! Cross-process file lock — kernel-released on crash (replaces v1 PID-file).
//!
//! Uses `fs4`'s `try_lock_exclusive` / `try_lock_shared` which wrap:
//!   • Unix:    `flock(2)` (BSD-style advisory)
//!   • Windows: `LockFileEx` with `LOCKFILE_FAIL_IMMEDIATELY` (mandatory range)
//!
//! Range is a single byte at offset 0, not the whole file, so on Windows other
//! processes can still open + read the JSON metadata for diagnostics without
//! tripping the mandatory-lock check.
//!
//! Two modes:
//!   • Exclusive — for writers (build_index path). Blocks all other lockers.
//!   • Shared    — for readers (query-only). Multiple shared lockers coexist;
//!                 exclusive cannot be acquired while any shared lock is held.
//!
//! NFS / SMB:
//!   `flock(2)` semantics on networked filesystems are unreliable (Linux
//!   silently emulates per-mount, macOS may no-op). We probe the filesystem
//!   type best-effort and warn on stderr — we do NOT refuse, because dev
//!   containers and CI runners frequently have networked $HOME and the user
//!   has no other option. The warning makes silent multi-locker bugs
//!   diagnosable. SQLite + LMDB take the same trade-off.
//!
//! Stale-lock recovery: when the holder process crashes, the kernel releases
//! the flock on fd-close. The JSON metadata sidecar may be left over but it's
//! purely informational — the lock state is the kernel's, not the file's.

use crate::cache::CacheLayout;
use ga_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Exclusive,
    Shared,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LockInfo {
    pub pid: u32,
    pub hostname: String,
    pub started_at_unix: u64,
    pub index_generation: String,
    pub mode: String,
}

#[derive(Debug)]
pub enum LockError {
    /// Another live instance holds an incompatible lock (exclusive vs anything,
    /// or shared vs exclusive request). Kept for backward-compat error text.
    Held {
        pid: u32,
        hostname: String,
        started_at_unix: u64,
    },
    /// I/O or serialization failure while manipulating the lock file.
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held {
                pid,
                hostname,
                started_at_unix,
            } => {
                let duration = humanize_duration(unix_now().saturating_sub(*started_at_unix));
                write!(
                    f,
                    "Another graphatlas instance (PID {pid} on {hostname}, started {duration} ago) \
                     is indexing this repo. Wait for completion or kill PID {pid}."
                )
            }
            Self::Io(s) => write!(f, "lock I/O: {s}"),
        }
    }
}

fn humanize_duration(secs: u64) -> String {
    match secs {
        0..=1 => "just now".to_string(),
        2..=59 => format!("{secs} seconds"),
        60..=3599 => format!("{} minutes", secs / 60),
        3600..=86_399 => format!("{} hours", secs / 3600),
        _ => format!("{} days", secs / 86_400),
    }
}

impl std::error::Error for LockError {}

impl From<LockError> for Error {
    fn from(value: LockError) -> Self {
        Error::Other(anyhow::anyhow!("{value}"))
    }
}

#[derive(Debug)]
pub struct LockFile {
    file: File,
    path: PathBuf,
    mode: LockMode,
}

impl LockFile {
    /// Try to take an exclusive lock — for writers (indexing path).
    ///
    /// Returns `Held` if any other process currently holds shared OR exclusive.
    pub fn try_acquire_exclusive(
        layout: &CacheLayout,
        index_generation: &str,
    ) -> std::result::Result<Self, LockError> {
        Self::try_acquire(layout, index_generation, LockMode::Exclusive)
    }

    /// Try to take a shared lock — for readers (query-only path).
    ///
    /// Multiple shared lockers coexist. Returns `Held` only when an exclusive
    /// lock is currently held.
    pub fn try_acquire_shared(
        layout: &CacheLayout,
        index_generation: &str,
    ) -> std::result::Result<Self, LockError> {
        Self::try_acquire(layout, index_generation, LockMode::Shared)
    }

    /// Backward-compat alias for v1 callers / tests. Equivalent to
    /// `try_acquire_exclusive` — preserves the original "exclusive or fail"
    /// semantics of the PID-file lock it replaces.
    pub fn acquire(
        layout: &CacheLayout,
        index_generation: &str,
    ) -> std::result::Result<Self, LockError> {
        Self::try_acquire_exclusive(layout, index_generation)
    }

    fn try_acquire(
        layout: &CacheLayout,
        index_generation: &str,
        mode: LockMode,
    ) -> std::result::Result<Self, LockError> {
        let path = layout.lock_pid();

        warn_if_networked_fs(&path);

        // Open (create if missing). 0600 enforced after — OpenOptions on Unix
        // honors umask, so `create(true)` may produce 0644 on some systems.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| LockError::Io(format!("open lock file {}: {e}", path.display())))?;
        // v1.5 PR2 audit bug #5 (AS-005): chmod failures are best-effort
        // (NFS/FUSE often refuse), but they MUST NOT be silently swallowed.
        // PR3 (Phase A4) will swap eprintln for tracing::warn!.
        if let Err(e) = chmod_0600_best_effort(&path) {
            log_chmod_failure(&path, &e);
        }

        // Acquire the kernel lock on a 1-byte range at offset 0.
        // fs4's API locks the whole file by handle on Unix and a 1-byte range
        // on Windows when given the `_range` variants — we use the simple
        // variants since BSD flock(2) is per-fd anyway and the Windows path
        // also uses byte-range under the hood for fs4 0.13+.
        let acquired = match mode {
            LockMode::Exclusive => FileExt::try_lock_exclusive(&file),
            LockMode::Shared => FileExt::try_lock_shared(&file),
        };

        match acquired {
            Ok(true) => {}
            Ok(false) => {
                let existing = read_lock(&path).ok();
                return Err(held_from(existing));
            }
            Err(e) => {
                return Err(LockError::Io(format!(
                    "flock {} ({mode:?}): {e}",
                    path.display()
                )));
            }
        }

        // Sidecar metadata for diagnostics. We MUST write into the existing
        // locked file handle — not via tmp-and-rename — because rename swaps
        // the inode, leaving our flock attached to an unlinked inode while
        // the path resolves to a fresh unlocked one. That broke the lock
        // semantics in v1.0 of this rewrite.
        if mode == LockMode::Exclusive {
            let info = LockInfo {
                pid: std::process::id(),
                hostname: current_hostname(),
                started_at_unix: unix_now(),
                index_generation: index_generation.to_string(),
                mode: "exclusive".to_string(),
            };
            if let Ok(bytes) = serde_json::to_vec_pretty(&info) {
                let _ = write_sidecar_in_place(&file, &bytes);
            }
        }

        Ok(Self { file, path, mode })
    }

    pub fn mode(&self) -> LockMode {
        self.mode
    }

    /// Downgrade an exclusive lock to shared so other readers can attach.
    /// Called by the writer after `commit_in_place` succeeds, so subsequent
    /// query-only instances can serve traffic against the now-committed cache.
    /// `flock(2)` and `LockFileEx` both treat a re-lock as a conversion.
    pub fn downgrade_to_shared(&mut self) -> Result<()> {
        if self.mode == LockMode::Shared {
            return Ok(());
        }
        match FileExt::try_lock_shared(&self.file) {
            Ok(true) => {
                self.mode = LockMode::Shared;
                Ok(())
            }
            Ok(false) => Err(Error::Other(anyhow::anyhow!(
                "lock downgrade refused — should be impossible from exclusive holder"
            ))),
            Err(e) => Err(Error::Other(anyhow::anyhow!("lock downgrade failed: {e}"))),
        }
    }

    /// v1.5 PR2 audit bug #4 (foundation S-001 AS-004) — rewrite the lock.pid
    /// sidecar JSON in place with a fresh `index_generation` value. Used by
    /// `Store::open_with_root_and_schema` after `begin_indexing_with_schema`
    /// mints the real build UUID; the lock was acquired earlier with the
    /// literal `"probe"` placeholder so this method swaps in the real value
    /// without releasing+reacquiring the kernel flock.
    ///
    /// No-op when this LockFile is in shared (reader) mode — only the writer
    /// owns the diagnostic sidecar contents.
    pub fn update_sidecar(&self, index_generation: &str) -> Result<()> {
        if self.mode != LockMode::Exclusive {
            return Ok(());
        }
        let info = LockInfo {
            pid: std::process::id(),
            hostname: current_hostname(),
            started_at_unix: unix_now(),
            index_generation: index_generation.to_string(),
            mode: "exclusive".to_string(),
        };
        let bytes = serde_json::to_vec_pretty(&info).map_err(|e| {
            Error::Other(anyhow::anyhow!(
                "serialize lock.pid sidecar (update): {e}"
            ))
        })?;
        write_sidecar_in_place(&self.file, &bytes).map_err(|e| {
            Error::Other(anyhow::anyhow!(
                "write lock.pid sidecar at {}: {e}",
                self.path.display()
            ))
        })
    }

    /// Explicit release (drops the kernel lock + removes the sidecar metadata
    /// file). Drop also releases the kernel lock as a fallback.
    pub fn release(self) -> Result<()> {
        // Best-effort: unlock first so peers see it released even if rm fails.
        let _ = FileExt::unlock(&self.file);
        if self.mode == LockMode::Exclusive && self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        // Drop runs naturally on the file handle next.
        Ok(())
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        // Kernel releases the lock when `self.file` closes — happens
        // automatically on drop. We only need to clean up the sidecar for
        // the exclusive holder so a fresh exclusive bid sees no stale info.
        let _ = FileExt::unlock(&self.file);
        if self.mode == LockMode::Exclusive {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn write_sidecar_in_place(file: &File, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = file;
    f.seek(SeekFrom::Start(0))?;
    f.set_len(0)?;
    f.write_all(bytes)?;
    f.flush()
}

fn read_lock(path: &Path) -> std::result::Result<LockInfo, LockError> {
    let bytes = std::fs::read(path).map_err(|e| LockError::Io(e.to_string()))?;
    serde_json::from_slice::<LockInfo>(&bytes).map_err(|e| LockError::Io(e.to_string()))
}

fn held_from(existing: Option<LockInfo>) -> LockError {
    match existing {
        Some(info) => LockError::Held {
            pid: info.pid,
            hostname: info.hostname,
            started_at_unix: info.started_at_unix,
        },
        None => LockError::Held {
            pid: 0,
            hostname: current_hostname(),
            started_at_unix: unix_now(),
        },
    }
}

fn current_hostname() -> String {
    hostname::get()
        .ok()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn chmod_0600_best_effort(path: &Path) -> Result<()> {
    crate::cache::chmod_0600(path)
}

/// v1.5 PR2 audit bug #5 (foundation S-001 AS-005) — emit a warning when
/// `chmod_0600_best_effort` fails. NFS/FUSE/SMB filesystems often refuse
/// the chmod call but the lock itself is still acquired correctly. We
/// surface the failure on stderr so operators can diagnose unexpected
/// permission state without crashing the writer.
///
/// **Note**: PR3 (Phase A4) will swap this `eprintln!` for
/// `tracing::warn!` once the workspace adopts `tracing`. Until then,
/// stderr is the visible side-channel.
///
/// Exposed `pub` so tests (and future integration callers) can invoke
/// the logging side-effect deterministically — avoids needing to provoke
/// a real OS chmod failure to test the contract.
pub fn log_chmod_failure(path: &Path, err: &dyn std::fmt::Display) {
    eprintln!(
        "warn: chmod 0600 best-effort failed for {}: {err}",
        path.display()
    );
}

/// Best-effort detection of NFS / SMB / CIFS / AFP. Logs a warning to stderr
/// when the lock file is on a networked filesystem so the user understands why
/// concurrent locking might silently misbehave. Does NOT refuse — see module
/// docs for rationale.
fn warn_if_networked_fs(lock_path: &Path) {
    let parent = lock_path.parent().unwrap_or(Path::new("/"));
    if let Some(fstype) = detect_fstype(parent) {
        let lower = fstype.to_ascii_lowercase();
        if lower.contains("nfs")
            || lower.contains("smb")
            || lower.contains("cifs")
            || lower.contains("afpfs")
        {
            eprintln!(
                "graphatlas: lock dir {} appears to be on filesystem '{fstype}' — \
                 file locks on networked filesystems are unreliable; concurrent \
                 graphatlas instances may silently race",
                parent.display()
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn detect_fstype(path: &Path) -> Option<String> {
    // Read /proc/mounts, find the longest mount-point prefix matching `path`,
    // return its fstype. Avoids any unsafe FFI by parsing the text file.
    let canon = std::fs::canonicalize(path).ok()?;
    let canon_str = canon.to_string_lossy().to_string();
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut parts = line.split_whitespace();
        let _src = parts.next()?;
        let mount_point = parts.next()?;
        let fstype = parts.next()?;
        if canon_str == mount_point || canon_str.starts_with(&format!("{mount_point}/")) {
            let len = mount_point.len();
            if best.as_ref().is_none_or(|(b, _)| len > *b) {
                best = Some((len, fstype.to_string()));
            }
        }
    }
    best.map(|(_, t)| t)
}

#[cfg(target_os = "macos")]
fn detect_fstype(path: &Path) -> Option<String> {
    // `stat -f %T <path>` prints fstype short name (e.g. "apfs", "nfs", "smbfs").
    let canon = std::fs::canonicalize(path).ok()?;
    let out = std::process::Command::new("/usr/bin/stat")
        .arg("-f")
        .arg("%T")
        .arg(&canon)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_fstype(_path: &Path) -> Option<String> {
    // Windows / BSD / others: skip — `~/.graphatlas` on UNC paths is rare,
    // and a misdetect on BSD is worse than no warning. Add per-OS detect
    // when a real user reports an issue.
    None
}
