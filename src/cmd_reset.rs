//! `graphatlas reset` — recovery for stuck/corrupted cache (v1.5 PR6.1
//! multi-mcp S-003).
//!
//! Default behaviour: probe the per-repo exclusive flock first; refuse
//! with a non-zero exit if any process holds it (challenge C-3 — wiping
//! a live writer's mmap'd graph.db corrupts the rebuild). `--force`
//! bypasses the probe explicitly; intended for use AFTER the operator
//! has confirmed the holder process is dead or has manually killed it.
//!
//! `--force` does NOT signal or kill any process — that constraint is
//! enforced by the CI grep lint declared in
//! `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-multi-mcp.md`.

use anyhow::{Context, Result};
use ga_index::cache::CacheLayout;
use ga_index::lock::{humanize_duration, read_lock, LockError, LockFile};
use ga_mcp::watcher::is_bench_fixture_path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn cmd_reset(repo: Option<PathBuf>, force: bool) -> Result<()> {
    let repo_root = match repo {
        Some(p) => p,
        None => std::env::current_dir().context("resolve cwd")?,
    };

    // AS-010 — bench fixture refusal applies in both default and --force
    // modes. Same guard as MCP boot + ga_reindex tool path.
    if is_bench_fixture_path(&repo_root) {
        return Err(anyhow::anyhow!(
            "reset refused: bench fixture path detected ({}). \
             Run on user project root.",
            repo_root.display()
        ));
    }

    let cache_root = default_cache_root()?;
    let layout = CacheLayout::for_repo(&cache_root, &repo_root);
    let cache_dir = layout.dir().to_path_buf();

    // AS-008/AS-009 — print the holder diagnostic from the sidecar
    // best-effort. No `kill(pid, 0)`, no hostname comparison, no
    // PID-recycling check — per Out-of-Scope.
    print_holder_diagnostic(&layout);

    if !force {
        // AS-008 default: try to acquire exclusive; refuse if held.
        match LockFile::try_acquire_exclusive(&layout, "reset") {
            Ok(lock) => {
                // Drop the lock immediately — we just wanted to confirm
                // nobody holds it. `remove_dir_all` below will wipe the
                // file the kernel flock sits on. Releasing first avoids
                // a hold-then-unlink ordering hazard.
                lock.release().ok();
            }
            Err(LockError::Held {
                pid,
                hostname,
                started_at_unix,
            }) => {
                let dur = humanize_duration(unix_now().saturating_sub(started_at_unix));
                tracing::warn!(
                    target: "graphatlas::reset",
                    holder_pid = pid,
                    holder_hostname = %hostname,
                    cache = %cache_dir.display(),
                    "reset refused: peer holds exclusive"
                );
                return Err(anyhow::anyhow!(
                    "Cache lock held by PID {pid} on {hostname} (started {dur} ago). \
                     Reset refused. If the holder is dead/stuck, retry with --force."
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!("reset: lock probe failed: {e}"));
            }
        }
    } else {
        eprintln!(
            "--force: skipping lock probe, may corrupt live writer at {}",
            cache_dir.display()
        );
        tracing::warn!(
            target: "graphatlas::reset",
            cache = %cache_dir.display(),
            "--force bypass on cache"
        );
    }

    // Wipe the cache dir. ENOENT (no cache yet) is fine — equivalent to a
    // fresh boot.
    match std::fs::remove_dir_all(&cache_dir) {
        Ok(()) => eprintln!("removed cache {}", cache_dir.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "cache {} did not exist; proceeding with reindex",
                cache_dir.display()
            );
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "reset: remove_dir_all({}): {e}",
                cache_dir.display()
            ));
        }
    }

    // Hand off to the existing reindex pipeline. `do_reindex` re-opens
    // the cache (which now lands on FreshBuild because we just wiped),
    // runs `reindex_in_place` → `build_index` → `commit_in_place`.
    crate::cmd_reindex::do_reindex(&repo_root, false)
}

fn print_holder_diagnostic(layout: &CacheLayout) {
    let lock_path = layout.lock_pid();
    if !lock_path.exists() {
        eprintln!("No lock.pid sidecar found — proceeding with reset");
        return;
    }
    match read_lock(&lock_path) {
        Ok(info) => {
            let dur = humanize_duration(unix_now().saturating_sub(info.started_at_unix));
            eprintln!(
                "Holder PID was {} on {} per sidecar (last seen {} ago)",
                info.pid, info.hostname, dur
            );
        }
        Err(e) => {
            eprintln!(
                "lock.pid sidecar present at {} but unreadable ({}) — proceeding with reset",
                lock_path.display(),
                e
            );
        }
    }
}

fn default_cache_root() -> Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("GRAPHATLAS_CACHE_DIR") {
        return Ok(PathBuf::from(override_dir));
    }
    let home = std::env::var("HOME").context("HOME not set; cannot resolve ~/.graphatlas")?;
    Ok(PathBuf::from(home).join(".graphatlas"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
