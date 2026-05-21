//! AS-049 — orphan grandchild recovery for `ga reindex` subprocesses.
//!
//! When `ga-server` crashes mid-reindex, the subprocess it spawned (a
//! `ga reindex` grandchild reparented to init) keeps running. The next
//! `ga-server` startup must scan `<cache_root>/<dir>/.reindex.pid`,
//! check whether the PID is still alive, and either:
//!   * adopt the running job → re-register in JobRegistry as Running
//!   * cleanup the dead PID file → mark cache `index_state: corrupt`
//!
//! `pid_alive` is injected so tests don't have to spawn real processes.

use std::path::{Path, PathBuf};

use crate::cache_state::CacheState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReindexPidFile {
    pub pid: u32,
    pub job_id: String,
    pub slug: String,
    pub started_at_unix: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// PID is alive — re-register the existing job in the registry.
    Adopt(ReindexPidFile),
    /// PID is dead — delete the pidfile + mark cache as Corrupt so the
    /// next GET /api/projects shows the badge.
    Cleanup { slug: String, cache_dir: PathBuf },
}

/// Scan every cache dir under `cache_root` for a `.reindex.pid` file.
/// Returns one `RecoveryAction` per file found. Caller applies the
/// actions (adopt → JobRegistry insert; cleanup → unlink + mark corrupt).
///
/// `pid_alive` is the probe (Spec D `cmd_ui::default_pid_alive` pattern
/// — `kill -0 PID` subprocess on unix). Tests inject a closure.
pub fn scan_orphan_pids(cache_root: &Path, pid_alive: impl Fn(u32) -> bool) -> Vec<RecoveryAction> {
    let mut actions = Vec::new();
    let entries = match std::fs::read_dir(cache_root) {
        Ok(e) => e,
        Err(_) => return actions,
    };
    for entry in entries.flatten() {
        let cache_dir = entry.path();
        if !cache_dir.is_dir() {
            continue;
        }
        let pid_path = cache_dir.join(".reindex.pid");
        let Ok(bytes) = std::fs::read(&pid_path) else {
            continue;
        };
        let Ok(pf) = serde_json::from_slice::<ReindexPidFile>(&bytes) else {
            // Corrupt pidfile counts the same as dead pid — schedule
            // cleanup so the cache doesn't get stuck.
            actions.push(RecoveryAction::Cleanup {
                slug: dir_slug(&cache_dir),
                cache_dir,
            });
            continue;
        };
        if pid_alive(pf.pid) {
            actions.push(RecoveryAction::Adopt(pf));
        } else {
            actions.push(RecoveryAction::Cleanup {
                slug: pf.slug,
                cache_dir,
            });
        }
    }
    actions
}

/// Probe `kill(pid, 0)` to check if a PID is alive. Mirrors
/// `graphatlas::cmd_ui::default_pid_alive` (the Spec D pattern) but
/// re-implemented here so ga-server doesn't take a dep on the binary
/// crate. Workspace forbids `unsafe` → subprocess fork/exec once per
/// call; cheap given how rarely this fires (only on stale-Building
/// cleanup or AS-049 startup scan).
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true // Conservative — refuse cleanup on non-Unix Phase 1.
    }
}

/// Read a `.reindex.pid` file from `cache_dir`. None if missing or
/// unparseable; the latter case is the same as "dead" for the caller.
pub fn read_pid_file(cache_dir: &Path) -> Option<ReindexPidFile> {
    let bytes = std::fs::read(cache_dir.join(".reindex.pid")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write the pidfile next to metadata.json. Called by the POST /reindex
/// handler after the launcher returns a PID — Spec A A-C9 invariant.
/// Atomic write (tmp + rename).
pub fn write_pid_file(cache_dir: &Path, pf: &ReindexPidFile) -> std::io::Result<()> {
    let path = cache_dir.join(".reindex.pid");
    let tmp = cache_dir.join(".reindex.pid.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(pf).unwrap())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Apply a Cleanup action — delete pidfile + flip metadata.json
/// `index_state` to `corrupt`. AS-049 dead-PID branch.
pub fn apply_cleanup(cache_dir: &Path) -> std::io::Result<()> {
    let pid_path = cache_dir.join(".reindex.pid");
    let _ = std::fs::remove_file(&pid_path);
    mark_metadata_corrupt(cache_dir)
}

/// Mutate `metadata.json` so `cache_state::lookup_cache_state` reports
/// `Corrupt`. Loose JSON edit (we don't have ga_index::IndexState::Corrupt
/// yet — Spec C-cross-8 only adds the layer on top of the existing
/// Building/Complete enum). The loose-probe fallback in `cache_state`
/// already recognizes `"index_state": "corrupt"`.
pub fn mark_metadata_corrupt(cache_dir: &Path) -> std::io::Result<()> {
    let md_path = cache_dir.join("metadata.json");
    let Ok(bytes) = std::fs::read(&md_path) else {
        return Ok(());
    };
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(());
    };
    if let Some(obj) = v.as_object_mut() {
        obj.insert("index_state".into(), serde_json::json!("corrupt"));
    }
    let tmp = cache_dir.join("metadata.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap())?;
    std::fs::rename(&tmp, &md_path)?;
    // Foundation-C8: ga-index refuses to open metadata.json with mode != 0600.
    // The default umask would leave the rewritten file at 0644 and the next
    // reindex subprocess would crash with `cache file has unsafe permissions`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&md_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn dir_slug(cache_dir: &Path) -> String {
    cache_dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|s| s.rsplit('-').next())
        .unwrap_or("")
        .to_string()
}

/// Resolve a slug back to its cache directory (mirrors the helper in
/// handlers/projects.rs and cache_state.rs — DRY candidate for
/// follow-up refactor).
pub fn find_cache_dir(cache_root: &Path, slug: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(cache_root).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(slug) || name == slug {
            return Some(p);
        }
    }
    None
}

/// Adjusted classifier used by tests / recovery flow.
pub fn classify_after_recovery(cache_root: &Path, slug: &str) -> CacheState {
    crate::cache_state::lookup_cache_state(cache_root, slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn seed_cache(root: &Path, dir_name: &str) -> PathBuf {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        let md = serde_json::json!({
            "schema_version": 5,
            "indexed_at": 1u64,
            "committed_at": 1u64,
            "repo_root": "/x",
            "index_state": "complete",
            "index_generation": "g",
            "indexed_root_hash": "",
            "graph_generation": 1,
            "cache_lang_set": []
        });
        std::fs::write(dir.join("metadata.json"), serde_json::to_vec(&md).unwrap()).unwrap();
        dir
    }

    #[test]
    fn no_pid_files_returns_empty() {
        let tmp = tempdir().unwrap();
        seed_cache(tmp.path(), "x-abc123");
        let actions = scan_orphan_pids(tmp.path(), |_| false);
        assert!(actions.is_empty());
    }

    #[test]
    fn dead_pid_yields_cleanup_action() {
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-dead00");
        write_pid_file(
            &dir,
            &ReindexPidFile {
                pid: 999_999,
                job_id: "job-dead".into(),
                slug: "dead00".into(),
                started_at_unix: 1,
            },
        )
        .unwrap();
        let actions = scan_orphan_pids(tmp.path(), |_| false);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], RecoveryAction::Cleanup { slug, .. } if slug == "dead00"));
    }

    #[test]
    fn alive_pid_yields_adopt_action() {
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-live00");
        write_pid_file(
            &dir,
            &ReindexPidFile {
                pid: 12345,
                job_id: "job-live".into(),
                slug: "live00".into(),
                started_at_unix: 1,
            },
        )
        .unwrap();
        let actions = scan_orphan_pids(tmp.path(), |_| true);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], RecoveryAction::Adopt(pf) if pf.slug == "live00"));
    }

    #[test]
    fn corrupt_pid_file_treated_as_dead() {
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-bad000");
        std::fs::write(dir.join(".reindex.pid"), b"not json").unwrap();
        let actions = scan_orphan_pids(tmp.path(), |_| true);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], RecoveryAction::Cleanup { .. }));
    }

    #[test]
    fn apply_cleanup_removes_pidfile_and_marks_corrupt() {
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-clean0");
        write_pid_file(
            &dir,
            &ReindexPidFile {
                pid: 999_999,
                job_id: "job-x".into(),
                slug: "clean0".into(),
                started_at_unix: 1,
            },
        )
        .unwrap();
        apply_cleanup(&dir).unwrap();
        assert!(!dir.join(".reindex.pid").exists());
        assert_eq!(
            classify_after_recovery(tmp.path(), "clean0"),
            CacheState::Corrupt
        );
    }

    // Regression: ga-index refuses metadata.json with mode != 0600. The
    // previous cleanup path used `fs::write` (umask → 0644), so the next
    // reindex subprocess crashed with "cache file has unsafe permissions".
    #[cfg(unix)]
    #[test]
    fn mark_metadata_corrupt_preserves_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-perm00");
        let md_path = dir.join("metadata.json");
        std::fs::set_permissions(&md_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        mark_metadata_corrupt(&dir).unwrap();

        let mode = std::fs::metadata(&md_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "metadata.json must stay 0600 after cleanup or ga-index refuses to open it"
        );
    }

    #[test]
    fn write_pid_file_is_atomic() {
        let tmp = tempdir().unwrap();
        let dir = seed_cache(tmp.path(), "x-atomi0");
        let pf = ReindexPidFile {
            pid: 1,
            job_id: "j".into(),
            slug: "atomi0".into(),
            started_at_unix: 1,
        };
        write_pid_file(&dir, &pf).unwrap();
        // .tmp must be gone after rename.
        assert!(!dir.join(".reindex.pid.tmp").exists());
        let roundtrip: ReindexPidFile =
            serde_json::from_slice(&std::fs::read(dir.join(".reindex.pid")).unwrap()).unwrap();
        assert_eq!(roundtrip, pf);
    }
}
