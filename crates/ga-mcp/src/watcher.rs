//! v1.5 PR8 — Layer 1 `.git/`-scoped FS watcher (triggers S-003).
//!
//! Watches a tight set of paths inside `.git/` (HEAD + index + refs/heads/)
//! so that external git operations — commits, checkouts, pulls, merges,
//! stash — trigger `ga_reindex` automatically. Catches changes from any
//! source (terminal git, IDE plugin, CI script), not just the agents
//! whose hook installer was wired up in PR7.
//!
//! **What this is NOT:**
//! - Not a repo-wide watcher (M-1 from the foundation spec rejects that
//!   because kernel inotify / FSEvents limits don't survive repo-wide
//!   recursive watches).
//! - Not a polling daemon — `RecommendedWatcher` uses inotify (Linux),
//!   FSEvents (macOS), or RDCW (Windows), with `PollWatcher` only as a
//!   fallback for exotic filesystems (NFS/FUSE/SMB) or inotify ENOSPC.
//!
//! **Layered with PR7 hook installer:** the hook installer catches edits
//! BEFORE commit; this watcher catches commits + checkouts + pulls. Both
//! layers fire `ga_reindex` through the same `McpContext::rebuild_via`
//! so the 200ms post-success cooldown (PR6.1d) absorbs any double-fire.

use std::path::{Path, PathBuf};

/// PR8 AS-008 + AS-008b — sentinel state files git creates while a
/// multi-step operation is mid-flight. When ANY of these exists, the
/// repo is in an in-progress rebase / merge / cherry-pick / bisect and
/// the working tree is NOT a coherent snapshot of code (it may contain
/// `<<<<<<<` conflict markers). Reindex during this window would feed
/// the parser garbage. Watcher defers until they're gone.
const GIT_OP_SENTINELS: &[&str] = &[
    "REBASE_HEAD",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "BISECT_LOG",
];

/// PR8 AS-008b — pure helper. Returns `true` if the repo at `repo_root`
/// is in an in-progress git operation (rebase / merge / cherry-pick /
/// bisect). Watcher pipeline calls this before dispatching a reindex
/// so a debounce timer firing mid-rebase doesn't kick off a build
/// against conflict-marker-laden source files.
///
/// Implementation: probe for any of the four sentinel files under
/// `<repo_root>/.git/`. Existence of even one is enough — we don't try
/// to distinguish abort-recoverable from terminal states; the watcher
/// just waits and retries.
pub fn is_git_op_in_progress(repo_root: &Path) -> bool {
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        return false;
    }
    GIT_OP_SENTINELS
        .iter()
        .any(|name| git_dir.join(name).exists())
        || git_dir.join("rebase-merge").is_dir()
        || git_dir.join("rebase-apply").is_dir()
}

/// PR8 AS-010 — defense-in-depth bench fixture refusal. Watcher init
/// fails (don't even start the tokio task) if `repo_root` resolves to a
/// path containing `/benches/fixtures/`. The MCP boot guard in
/// `mcp_cmd::prepare_store_for_mcp` already refuses to serve on bench
/// fixtures; this is a second line so a test fixture that bypasses the
/// boot guard can't end up mutating canonical submodule HEAD via the
/// watcher's reindex dispatch.
pub fn is_bench_fixture_path(repo_root: &Path) -> bool {
    let canonical = std::fs::canonicalize(repo_root).ok();
    let probe = canonical.as_deref().unwrap_or(repo_root);
    let s = probe.to_string_lossy().replace('\\', "/");
    s.contains("/benches/fixtures/")
}

/// PR8 AS-007 — paths inside `.git/` the watcher subscribes to. Kept
/// small on purpose: HEAD covers checkouts + commits + reset, index
/// covers stage/unstage, refs/heads/ covers branch renames + pull
/// fast-forwards. One inotify slot total (refs/heads/ is a single dir
/// watch; we don't recurse).
///
/// Returns the absolute paths that should be passed to
/// `notify::Watcher::watch`. Missing paths are silently skipped so the
/// watcher can survive a freshly-cloned repo that hasn't yet committed.
pub fn watch_targets(repo_root: &Path) -> Vec<PathBuf> {
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        return Vec::new();
    }
    let candidates = [
        git_dir.join("HEAD"),
        git_dir.join("index"),
        git_dir.join("refs").join("heads"),
    ];
    candidates.into_iter().filter(|p| p.exists()).collect()
}

/// PR8 AS-010 + watcher init — verify `repo_root` is something we are
/// willing to watch. Returns the validated `.git/` directory on success.
pub fn validate_repo_root(repo_root: &Path) -> Result<PathBuf, WatcherInitError> {
    if is_bench_fixture_path(repo_root) {
        return Err(WatcherInitError::BenchFixtureRefused {
            path: repo_root.to_path_buf(),
        });
    }
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        return Err(WatcherInitError::NotAGitRepo {
            path: repo_root.to_path_buf(),
        });
    }
    Ok(git_dir)
}

#[derive(Debug)]
pub enum WatcherInitError {
    /// AS-010 — repo_root is a bench fixture; watcher refuses to start.
    BenchFixtureRefused { path: PathBuf },
    /// Not a git repository (no `.git/` directory). Watcher is a no-op
    /// here; Layer 3 staleness gate (PR5) still covers correctness.
    NotAGitRepo { path: PathBuf },
    /// AS-011 — inotify ENOSPC or similar. Caller should fall back to
    /// `PollWatcher` and re-init.
    InotifyExhausted { source: String },
}

impl std::fmt::Display for WatcherInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BenchFixtureRefused { path } => write!(
                f,
                "watcher refused: bench fixture path {} would corrupt M1/M2/M3 gates",
                path.display()
            ),
            Self::NotAGitRepo { path } => write!(
                f,
                "watcher no-op: {} is not a git repository (.git/ missing)",
                path.display()
            ),
            Self::InotifyExhausted { source } => write!(
                f,
                "watcher init: inotify exhausted ({source}); falling back to PollWatcher"
            ),
        }
    }
}

impl std::error::Error for WatcherInitError {}

/// PR8 AS-011 — classify a `notify::Error` to decide whether we should
/// fall back from `RecommendedWatcher` to `PollWatcher`. Returns `true`
/// for Linux inotify ENOSPC and any "max watches exceeded" message.
pub fn should_fallback_to_polling(err_msg: &str) -> bool {
    let lower = err_msg.to_lowercase();
    lower.contains("enospc")
        || lower.contains("no space left")
        || lower.contains("max_user_watches")
        || lower.contains("too many open files")
        || lower.contains("limit reached")
}

// =====================================================================
// PR8 AS-007 — runtime: watcher → debouncer → reindex dispatch
// =====================================================================

use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// PR8 AS-007 spec literal — 500ms idle window after the last `.git/`
/// event before the watcher fires a coalesced reindex. Tuned to absorb
/// `git rebase -i` HEAD thrash + atomic-rename pairs on `.git/index`.
pub const DEBOUNCE_MS: u64 = 500;

/// PR8 AS-008 — retry interval used while a git op is in progress.
/// Watcher re-arms every 2s checking whether the sentinel files cleared.
pub const GIT_OP_RECHECK_MS: u64 = 2_000;

/// Signal emitted by the watcher loop. Tests + production both observe
/// this stream via the dispatcher callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEvent {
    /// Coalesced reindex trigger after the debounce window expired AND
    /// no git op was in progress.
    ReindexFired,
    /// Debounce fired but `is_git_op_in_progress` returned true; reindex
    /// deferred. Watcher re-arms a recheck timer.
    DeferredGitOp,
}

/// PR8 AS-007 — synchronous run loop. Consumes notify events, debounces
/// them, checks git-op sentinels, and invokes `dispatch` once per
/// coalesced burst. Returns when the underlying notify event channel is
/// closed (test harness drops the watcher; production drops it on MCP
/// shutdown).
///
/// `dispatch` is called BOTH for `ReindexFired` (caller invokes
/// `ga_reindex`) and `DeferredGitOp` (caller logs but does nothing).
/// Keeping it one callback simplifies the test harness — production
/// only cares about `ReindexFired`.
///
/// The loop is intentionally tokio-free so it can run in a plain
/// `std::thread` from tests AND from a tokio `spawn_blocking` in the
/// production MCP path. The notify crate's recommended watchers are
/// thread-based already; an async wrapper would buy nothing here.
pub fn run_watch_loop<F>(
    repo_root: &Path,
    rx: mpsc::Receiver<notify::Result<Event>>,
    mut dispatch: F,
) where
    F: FnMut(WatcherEvent),
{
    let mut pending_since: Option<Instant> = None;
    loop {
        // Wait for either (a) a notify event, (b) the debounce window
        // expiring with a pending event, or (c) the channel closing.
        let recv_timeout = match pending_since {
            Some(since) => {
                let elapsed = since.elapsed();
                let window = Duration::from_millis(DEBOUNCE_MS);
                if elapsed >= window {
                    Duration::from_millis(0)
                } else {
                    window - elapsed
                }
            }
            None => Duration::from_millis(GIT_OP_RECHECK_MS),
        };
        match rx.recv_timeout(recv_timeout) {
            Ok(Ok(ev)) => {
                if is_relevant_event(&ev) {
                    pending_since = Some(Instant::now());
                }
            }
            Ok(Err(_e)) => {
                // notify reported an internal error — keep running; the
                // event stream may still recover. Caller is expected to
                // have surfaced ENOSPC at init time and switched to
                // PollWatcher in that case.
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(since) = pending_since {
                    if since.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                        if is_git_op_in_progress(repo_root) {
                            dispatch(WatcherEvent::DeferredGitOp);
                            // Hold the pending_since timestamp so the
                            // next iteration's recheck timer fires after
                            // GIT_OP_RECHECK_MS rather than another
                            // DEBOUNCE_MS window.
                            pending_since = Some(
                                Instant::now() - Duration::from_millis(DEBOUNCE_MS)
                                    + Duration::from_millis(GIT_OP_RECHECK_MS),
                            );
                        } else {
                            dispatch(WatcherEvent::ReindexFired);
                            pending_since = None;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// PR8 AS-007 — classify a notify Event. We only care about file-content
/// changes inside `.git/` (HEAD, index, refs/heads/<branch>). Metadata
/// events (mode/atime changes) are noise.
fn is_relevant_event(ev: &Event) -> bool {
    matches!(
        ev.kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
    )
}

/// PR8 AS-007 + AS-011 — spawn a `RecommendedWatcher` against the
/// validated `.git/` targets. Returns the watcher handle (caller keeps
/// it alive for the lifetime of the watch) + the receiver channel for
/// `run_watch_loop`.
///
/// On notify init failure with an ENOSPC-class error, the caller should
/// catch the returned `Err` and re-init using `PollWatcher` (AS-011
/// transparent fallback).
pub fn spawn_recommended_watcher(
    repo_root: &Path,
) -> Result<
    (
        notify::RecommendedWatcher,
        mpsc::Receiver<notify::Result<Event>>,
    ),
    notify::Error,
> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::RecommendedWatcher::new(tx, notify::Config::default())?;
    for target in watch_targets(repo_root) {
        let mode = if target.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher.watch(&target, mode)?;
    }
    Ok((watcher, rx))
}

/// Owns the watcher handle + the joined dispatch thread for the
/// lifetime of the MCP server. Drop releases the notify backend AND
/// closes the run-loop channel, signaling the dispatch thread to exit.
pub struct WatcherGuard {
    _watcher: notify::RecommendedWatcher,
}

/// PR8 AS-007 + AS-008b + AS-010 — production entry. Validates the
/// repo, spawns a `RecommendedWatcher`, and runs the dispatcher thread
/// that calls `ga_reindex` via `ctx.rebuild_via` on each coalesced
/// burst. Returns a `WatcherGuard` whose Drop tears the watcher down on
/// MCP shutdown.
///
/// On `WatcherInitError::BenchFixtureRefused` or
/// `WatcherInitError::NotAGitRepo` the function returns `None` rather
/// than `Err` so the MCP boot can continue gracefully — the watcher is
/// a polish layer, not a correctness requirement (Layer 3 staleness
/// gate from PR5 still covers correctness).
pub fn spawn_l1_watcher(
    ctx: crate::context::McpContext,
    repo_root: PathBuf,
) -> Option<WatcherGuard> {
    let _git_dir = match validate_repo_root(&repo_root) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "L1 watcher not started");
            return None;
        }
    };
    let (watcher, rx) = match spawn_recommended_watcher(&repo_root) {
        Ok(pair) => pair,
        Err(e) => {
            let msg = e.to_string();
            if should_fallback_to_polling(&msg) {
                tracing::warn!(
                    "L1 watcher: inotify exhausted ({msg}); PollWatcher fallback \
                     deferred — L3 staleness gate still covers correctness"
                );
            } else {
                tracing::warn!("L1 watcher init failed: {msg}");
            }
            return None;
        }
    };
    let repo_for_loop = repo_root.clone();
    let repo_for_dispatch = repo_root.clone();
    std::thread::spawn(move || {
        run_watch_loop(&repo_for_loop, rx, move |ev| match ev {
            WatcherEvent::ReindexFired => {
                let _ = dispatch_reindex(&ctx, &repo_for_dispatch);
            }
            WatcherEvent::DeferredGitOp => {
                tracing::info!(
                    repo = %repo_for_dispatch.display(),
                    "L1 watcher: git op in progress; deferring reindex until cleared"
                );
            }
        });
    });
    Some(WatcherGuard { _watcher: watcher })
}

/// Internal dispatch path for L1 watcher → ga_reindex. Mirrors the
/// `tools::reindex::call` flow but bypasses the rmcp transport since
/// the watcher dispatches in-process. Errors are logged and dropped;
/// the watcher MUST NOT crash the MCP on transient reindex failures.
fn dispatch_reindex(ctx: &crate::context::McpContext, repo_root: &Path) -> ga_core::Result<()> {
    let cache_dir = ctx.store().layout().dir().to_path_buf();
    let lock_arc = ctx.reindex_lock_for(&cache_dir);
    let _guard = lock_arc.lock().expect("L1 reindex mutex");
    if let Err(e) = ctx.check_reindex_cooldown(&cache_dir) {
        tracing::debug!("L1 watcher: cooldown active ({e}); skipping");
        return Ok(());
    }
    let result = ctx.rebuild_via(|store| {
        let inner = std::path::PathBuf::from(&store.metadata().repo_root);
        let mut fresh = store
            .reindex_in_place(&inner)
            .map_err(|e| ga_core::Error::Other(anyhow::anyhow!("reindex_in_place: {e}")))?;
        ga_query::indexer::build_index(&fresh, &inner)
            .map_err(|e| ga_core::Error::Other(anyhow::anyhow!("build_index: {e}")))?;
        fresh
            .commit_in_place()
            .map_err(|e| ga_core::Error::Other(anyhow::anyhow!("commit_in_place: {e}")))?;
        Ok(fresh)
    });
    match result {
        Ok(new_store) => {
            ctx.record_reindex_success(&cache_dir);
            tracing::info!(
                repo = %repo_root.display(),
                generation = new_store.metadata().graph_generation,
                "L1 watcher: reindex complete"
            );
            Ok(())
        }
        Err(e) => {
            tracing::warn!(error = %e, "L1 watcher: reindex dispatch failed");
            Err(e)
        }
    }
}
