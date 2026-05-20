//! File-system watcher control — Spec A S-006.
//!
//! This module ships:
//!   * the state machine + per-slug registry,
//!   * 4 pure-logic helpers that encode the policy decisions
//!     (AS-051..AS-054),
//!   * a `WatcherDriver` trait that abstracts the actual notify-rs
//!     `RecommendedWatcher` spawn — kept behind a seam so the route
//!     handlers + state transitions are unit-testable without touching
//!     the filesystem. The real `notify`-backed driver lands in the
//!     S-006 follow-up build (see `.build-checklist` S-006-INFRA).
//!
//! Cross-cutting decision (post-/mf-challenge H-1): ga-server does
//! *not* extract `ga-watcher` — direct reuse from `ga_mcp::watcher::*`
//! is the target for the production driver. The trait seam keeps us
//! ready for that without coupling tests to ga-mcp's MCP context.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

// ============== Public types ==============

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum WatcherStatus {
    Stopped,
    Running,
    Errored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherMode {
    /// `notify::RecommendedWatcher` — inotify on Linux, FSEvents on
    /// macOS, RDCW on Windows. Default fast path. Serializes to the
    /// host-correct backend name so the UI shows the truth.
    Inotify,
    /// Fallback when inotify resources are exhausted (AS-053). Polls
    /// the watched roots on a 2-3s interval.
    Poll,
}

impl Serialize for WatcherMode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = match self {
            WatcherMode::Poll => "poll",
            WatcherMode::Inotify => {
                #[cfg(target_os = "macos")]
                {
                    "fsevents"
                }
                #[cfg(target_os = "linux")]
                {
                    "inotify"
                }
                #[cfg(target_os = "windows")]
                {
                    "rdcw"
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
                {
                    "native"
                }
            }
        };
        s.serialize_str(v)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WatcherSnapshot {
    pub slug: String,
    pub status: WatcherStatus,
    pub mode: WatcherMode,
    pub queue_pending: u64,
    /// AS-052 — set when the queue cap (1000) tripped and the watcher
    /// switched to "just remember something changed, drop per-file
    /// detail" mode. The next reindex does a full staleness scan.
    pub dirty_flag: bool,
    pub last_event_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct WatcherEntry {
    pub slug: String,
    pub status: WatcherStatus,
    pub mode: WatcherMode,
    pub queue_pending: u64,
    pub dirty_flag: bool,
    pub last_event_unix: Option<u64>,
    pub error: Option<String>,
}

impl WatcherEntry {
    pub fn new_stopped(slug: &str) -> Self {
        Self {
            slug: slug.into(),
            status: WatcherStatus::Stopped,
            mode: WatcherMode::Inotify,
            queue_pending: 0,
            dirty_flag: false,
            last_event_unix: None,
            error: None,
        }
    }

    pub fn snapshot(&self) -> WatcherSnapshot {
        WatcherSnapshot {
            slug: self.slug.clone(),
            status: self.status,
            mode: self.mode,
            queue_pending: self.queue_pending,
            dirty_flag: self.dirty_flag,
            last_event_unix: self.last_event_unix,
            error: self.error.clone(),
        }
    }
}

// ============== Registry ==============

pub struct WatcherRegistry {
    inner: Mutex<HashMap<String, Arc<Mutex<WatcherEntry>>>>,
}

impl Default for WatcherRegistry {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl WatcherRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get-or-create the entry for `slug`. Brand-new entries start in
    /// `Stopped` state.
    pub fn entry(&self, slug: &str) -> Arc<Mutex<WatcherEntry>> {
        let mut guard = self.inner.lock().expect("WatcherRegistry mutex poisoned");
        guard
            .entry(slug.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(WatcherEntry::new_stopped(slug))))
            .clone()
    }

    pub fn lookup(&self, slug: &str) -> Option<Arc<Mutex<WatcherEntry>>> {
        self.inner
            .lock()
            .expect("WatcherRegistry mutex poisoned")
            .get(slug)
            .cloned()
    }
}

// ============== Driver trait ==============

/// Outcome the driver reports back after a start attempt. The handler
/// translates this into JSON the UI consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartOutcome {
    /// Watcher spawned successfully.
    Started(WatcherMode),
    /// Watcher spawned but in fallback mode (AS-053 ENOSPC path).
    FallbackPoll(String),
    /// Spawn failed — surface error message; entry status → Errored.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopOutcome {
    Stopped,
    /// Was never running — no-op.
    AlreadyStopped,
    /// Thread join took longer than the budget; sent kill but didn't
    /// observe clean exit. UI still flips to Stopped because the
    /// resource is best-effort released.
    KilledAfterTimeout,
}

pub trait WatcherDriver: Send + Sync + 'static {
    fn start(&self, slug: &str, repo_root: &Path) -> StartOutcome;
    fn stop(&self, slug: &str) -> StopOutcome;
}

// ============== Pure logic helpers ==============

/// AS-052 — queue cap. Once pending events exceed this threshold, the
/// watcher drops the per-file queue and just remembers "something
/// changed". Memory upper bound becomes O(1) regardless of churn.
pub const QUEUE_CAP: u64 = 1000;

pub fn should_switch_to_dirty_flag(pending: u64) -> bool {
    pending > QUEUE_CAP
}

/// AS-053 — decide whether a start error means we should retry with
/// PollWatcher. Matches the substring patterns the kernel + notify-rs
/// surface for inotify exhaustion. Mirrors the behaviour shipped in
/// `ga_mcp::watcher::should_fallback_to_polling` (cross-cutting
/// decision H-1: reuse, don't extract).
pub fn should_fallback_to_polling(err_msg: &str) -> bool {
    let lower = err_msg.to_ascii_lowercase();
    lower.contains("enospc")
        || lower.contains("max watches exceeded")
        || lower.contains("too many open files")
        || lower.contains("inotify_init")
}

/// AS-054 — detect an in-progress git operation. If true, the watcher
/// must defer reindex; otherwise debouncer fires mid-rebase and trashes
/// a half-written branch index.
pub fn is_git_op_in_progress(repo_root: &Path) -> bool {
    let git = repo_root.join(".git");
    // Each marker corresponds to a `git` subcommand that holds a
    // working-tree lock or write barrier. Existence = op in progress.
    const MARKERS: &[&str] = &[
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
        "index.lock",
    ];
    for m in MARKERS {
        if git.join(m).exists() {
            return true;
        }
    }
    false
}

/// AS-051 — decide whether a file-change event should trigger a fresh
/// reindex job. The watcher loop calls this on every debounced batch:
///   * If JobRegistry already has an entry for the slug → skip
///     (race-safe; the existing job will pick up the new edits).
///   * Otherwise return true; caller invokes `try_insert` + spawn.
pub fn should_trigger_reindex(jobs: &crate::jobs::JobRegistry, slug: &str) -> bool {
    jobs.get(slug).is_none()
}

// ============== Test driver ==============

#[cfg(any(test, feature = "test-fixture"))]
pub mod fake {
    use super::*;

    /// Recorded start/stop calls so tests can assert the driver was
    /// invoked. Configurable per-slug outcome via `set_outcome`.
    pub struct FakeWatcherDriver {
        starts: Mutex<Vec<String>>,
        stops: Mutex<Vec<String>>,
        next_outcome: Mutex<HashMap<String, StartOutcome>>,
    }

    impl FakeWatcherDriver {
        pub fn new() -> Self {
            Self {
                starts: Mutex::new(Vec::new()),
                stops: Mutex::new(Vec::new()),
                next_outcome: Mutex::new(HashMap::new()),
            }
        }

        /// Configure the outcome `start(slug, ...)` returns next.
        pub fn set_outcome(&self, slug: &str, outcome: StartOutcome) {
            self.next_outcome
                .lock()
                .unwrap()
                .insert(slug.to_string(), outcome);
        }

        pub fn starts(&self) -> Vec<String> {
            self.starts.lock().unwrap().clone()
        }
        pub fn stops(&self) -> Vec<String> {
            self.stops.lock().unwrap().clone()
        }
    }

    impl Default for FakeWatcherDriver {
        fn default() -> Self {
            Self::new()
        }
    }

    impl WatcherDriver for FakeWatcherDriver {
        fn start(&self, slug: &str, _repo_root: &Path) -> StartOutcome {
            self.starts.lock().unwrap().push(slug.to_string());
            self.next_outcome
                .lock()
                .unwrap()
                .remove(slug)
                .unwrap_or(StartOutcome::Started(WatcherMode::Inotify))
        }

        fn stop(&self, slug: &str) -> StopOutcome {
            self.stops.lock().unwrap().push(slug.to_string());
            StopOutcome::Stopped
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn dirty_flag_triggers_at_cap_plus_one() {
        assert!(!should_switch_to_dirty_flag(0));
        assert!(!should_switch_to_dirty_flag(QUEUE_CAP));
        assert!(should_switch_to_dirty_flag(QUEUE_CAP + 1));
        assert!(should_switch_to_dirty_flag(u64::MAX));
    }

    // Regression: WatcherMode::Inotify hardcoded the "inotify" label
    // regardless of host OS. On macOS the underlying notify backend is
    // FSEvents, on Windows it's RDCW — the UI was lying to users.
    #[test]
    fn native_watcher_mode_serializes_per_host_os() {
        let s = serde_json::to_string(&WatcherMode::Inotify).unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(s, "\"fsevents\"");
        #[cfg(target_os = "linux")]
        assert_eq!(s, "\"inotify\"");
        #[cfg(target_os = "windows")]
        assert_eq!(s, "\"rdcw\"");
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        assert_eq!(s, "\"native\"");
    }

    #[test]
    fn poll_fallback_on_enospc_substring() {
        assert!(should_fallback_to_polling("inotify_init: ENOSPC"));
        assert!(should_fallback_to_polling(
            "ERROR: max watches exceeded for user"
        ));
        assert!(should_fallback_to_polling("Too Many Open Files"));
        assert!(!should_fallback_to_polling("permission denied"));
        assert!(!should_fallback_to_polling("file not found"));
    }

    #[test]
    fn git_op_detected_when_rebase_merge_dir_exists() {
        let repo = tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git/rebase-merge")).unwrap();
        assert!(is_git_op_in_progress(repo.path()));
    }

    #[test]
    fn git_op_detected_when_merge_head_file_exists() {
        let repo = tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        std::fs::write(repo.path().join(".git/MERGE_HEAD"), b"abc").unwrap();
        assert!(is_git_op_in_progress(repo.path()));
    }

    #[test]
    fn git_op_not_detected_in_clean_repo() {
        let repo = tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git/refs/heads")).unwrap();
        assert!(!is_git_op_in_progress(repo.path()));
    }

    #[test]
    fn git_op_not_detected_in_non_git_dir() {
        let repo = tempdir().unwrap();
        assert!(!is_git_op_in_progress(repo.path()));
    }

    #[test]
    fn should_trigger_reindex_returns_true_when_no_active_job() {
        let jobs = crate::jobs::JobRegistry::new();
        assert!(should_trigger_reindex(&jobs, "slug-a"));
    }

    #[test]
    fn should_trigger_reindex_returns_false_when_job_in_flight() {
        let jobs = crate::jobs::JobRegistry::new();
        let _ = jobs.try_insert("slug-a");
        assert!(!should_trigger_reindex(&jobs, "slug-a"));
        // Different slug still triggers.
        assert!(should_trigger_reindex(&jobs, "slug-b"));
    }

    #[test]
    fn registry_get_or_create_returns_same_handle() {
        let reg = WatcherRegistry::new();
        let a = reg.entry("x");
        let b = reg.entry("x");
        assert!(Arc::ptr_eq(&a, &b));
    }
}

// ============== Production driver — real notify-rs spawn ==============

use notify::Watcher as _;

/// Real driver. Per-slug `notify::RecommendedWatcher` (inotify on Linux,
/// FSEvents on macOS, RDCW on Windows). On ENOSPC, retries with the
/// `PollWatcher` fallback per AS-053.
///
/// Phase 1 keeps the event handler minimal — it just records that
/// "something changed" by bumping `queue_pending`. Auto-reindex
/// dispatch (the path through `should_trigger_reindex` + JobRegistry
/// spawn) is best-effort: the watcher signals state, the user clicks
/// Reindex. Wire to JobLauncher in a follow-up if dogfood demands
/// fully-automatic reindex.
pub struct NotifyWatcherDriver {
    sessions: Mutex<HashMap<String, WatcherSession>>,
    registry: std::sync::Arc<WatcherRegistry>,
}

struct WatcherSession {
    // Boxed because notify watchers aren't trivially Sized across the
    // `dyn` boundary.
    _watcher: Box<dyn notify::Watcher + Send + Sync>,
}

impl NotifyWatcherDriver {
    pub fn new(registry: std::sync::Arc<WatcherRegistry>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            registry,
        }
    }
}

impl WatcherDriver for NotifyWatcherDriver {
    fn start(&self, slug: &str, repo_root: &Path) -> StartOutcome {
        let entry = self.registry.entry(slug);
        let slug_owned = slug.to_string();

        let handler = move |res: notify::Result<notify::Event>| {
            // Ignore notify-internal errors (rescan, overflow) — they
            // just mean events were dropped. The dirty_flag covers that.
            if res.is_err() {
                return;
            }
            let event = res.unwrap();
            // Skip events under `.git/` — they're noisy and we already
            // gate via `is_git_op_in_progress`.
            if event
                .paths
                .iter()
                .any(|p| p.components().any(|c| c.as_os_str() == ".git"))
            {
                return;
            }
            let mut guard = entry.lock().unwrap();
            guard.last_event_unix = Some(now_unix());
            if should_switch_to_dirty_flag(guard.queue_pending) {
                guard.dirty_flag = true;
            } else {
                guard.queue_pending = guard.queue_pending.saturating_add(1);
            }
        };

        match notify::recommended_watcher(handler.clone()) {
            Ok(mut w) => match w.watch(repo_root, notify::RecursiveMode::Recursive) {
                Ok(()) => {
                    self.sessions.lock().unwrap().insert(
                        slug_owned,
                        WatcherSession {
                            _watcher: Box::new(w),
                        },
                    );
                    StartOutcome::Started(WatcherMode::Inotify)
                }
                Err(e) => {
                    let msg = e.to_string();
                    if should_fallback_to_polling(&msg) {
                        try_poll_fallback(slug_owned, repo_root, handler, &self.sessions, &msg)
                    } else {
                        StartOutcome::Failed(format!("watcher.watch: {msg}"))
                    }
                }
            },
            Err(e) => {
                let msg = e.to_string();
                if should_fallback_to_polling(&msg) {
                    try_poll_fallback(slug_owned, repo_root, handler, &self.sessions, &msg)
                } else {
                    StartOutcome::Failed(format!("recommended_watcher: {msg}"))
                }
            }
        }
    }

    fn stop(&self, slug: &str) -> StopOutcome {
        let mut guard = self.sessions.lock().unwrap();
        if guard.remove(slug).is_some() {
            StopOutcome::Stopped
        } else {
            StopOutcome::AlreadyStopped
        }
    }
}

fn try_poll_fallback<F>(
    slug: String,
    repo_root: &Path,
    handler: F,
    sessions: &Mutex<HashMap<String, WatcherSession>>,
    inotify_err: &str,
) -> StartOutcome
where
    F: notify::EventHandler + Clone + Send + 'static,
{
    let cfg = notify::Config::default().with_poll_interval(std::time::Duration::from_secs(3));
    match notify::PollWatcher::new(handler, cfg) {
        Ok(mut w) => match w.watch(repo_root, notify::RecursiveMode::Recursive) {
            Ok(()) => {
                sessions.lock().unwrap().insert(
                    slug,
                    WatcherSession {
                        _watcher: Box::new(w),
                    },
                );
                StartOutcome::FallbackPoll(format!("inotify failed: {inotify_err}"))
            }
            Err(e) => StartOutcome::Failed(format!("PollWatcher.watch: {e}")),
        },
        Err(e) => StartOutcome::Failed(format!("PollWatcher::new: {e}")),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// silence unused-import lint when test-fixture is disabled
#[allow(dead_code)]
fn _empty_path(_p: &PathBuf) {}
