//! v1.5 PR8 — `.git/`-scoped FS watcher end-to-end smoke tests.
//!
//! Pure helpers are covered in `watcher_helpers.rs`. This file exercises
//! the runtime pipeline: real notify backend → debouncer → dispatch
//! callback. Tests run a `RecommendedWatcher` against a TempDir repo
//! and verify the callback fires (AS-007) and that an in-progress
//! rebase sentinel suppresses dispatch (AS-008/008b).

use ga_mcp::watcher::{run_watch_loop, spawn_recommended_watcher, WatcherEvent, DEBOUNCE_MS};
use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn init_repo(tmp: &TempDir) -> std::path::PathBuf {
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join(".git").join("refs").join("heads")).unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(repo.join(".git").join("index"), b"DIRC").unwrap();
    fs::write(
        repo.join(".git").join("refs").join("heads").join("main"),
        "abc1234\n",
    )
    .unwrap();
    repo
}

/// Drive the watch loop on a background thread + collect dispatched
/// events. Returns the collector handle + a shutdown channel.
fn spawn_loop(
    repo_root: std::path::PathBuf,
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
) -> Arc<Mutex<Vec<WatcherEvent>>> {
    let events: Arc<Mutex<Vec<WatcherEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    thread::spawn(move || {
        run_watch_loop(&repo_root, rx, move |ev| {
            events_clone.lock().unwrap().push(ev);
        });
    });
    events
}

fn wait_for<F: Fn() -> bool>(timeout: Duration, predicate: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

// =====================================================================
// AS-007 — HEAD change fires a single coalesced reindex
// =====================================================================

#[test]
fn as_007_head_modification_fires_reindex_after_debounce() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let (_watcher, rx) = spawn_recommended_watcher(&repo).expect("watcher init must succeed");
    let events = spawn_loop(repo.clone(), rx);

    // Touch HEAD — simulates `git commit` rewriting the ref.
    thread::sleep(Duration::from_millis(50));
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/feat\n").unwrap();

    let total_wait = Duration::from_millis(DEBOUNCE_MS + 1500);
    let fired = wait_for(total_wait, || {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, WatcherEvent::ReindexFired))
    });
    let collected = events.lock().unwrap().clone();
    assert!(
        fired,
        "AS-007: HEAD modification must fire ReindexFired within {}ms; got {collected:?}",
        total_wait.as_millis()
    );
}

#[test]
fn as_007_rapid_burst_coalesces_into_single_reindex() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let (_watcher, rx) = spawn_recommended_watcher(&repo).expect("watcher init must succeed");
    let events = spawn_loop(repo.clone(), rx);

    thread::sleep(Duration::from_millis(50));
    // Five HEAD rewrites in 250ms (well within the 500ms debounce).
    for i in 0..5 {
        fs::write(
            repo.join(".git").join("HEAD"),
            format!("ref: refs/heads/burst-{i}\n"),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(40));
    }

    // Wait long enough for the debounce window plus filesystem-event
    // settle latency (CI runners can be slow).
    thread::sleep(Duration::from_millis(DEBOUNCE_MS + 1500));
    let collected = events.lock().unwrap().clone();
    let fires = collected
        .iter()
        .filter(|e| matches!(e, WatcherEvent::ReindexFired))
        .count();
    assert_eq!(
        fires, 1,
        "AS-007: 5 rapid HEAD rewrites must coalesce into 1 reindex; got {collected:?}"
    );
}

// =====================================================================
// AS-008 — in-progress git op defers reindex
// =====================================================================

#[test]
fn as_008b_rebase_head_sentinel_suppresses_reindex_then_resumes_after_cleared() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    // Plant the rebase-merge sentinel BEFORE the watcher starts.
    fs::write(repo.join(".git").join("REBASE_HEAD"), "abc1234\n").unwrap();
    let (_watcher, rx) = spawn_recommended_watcher(&repo).expect("watcher init must succeed");
    let events = spawn_loop(repo.clone(), rx);

    thread::sleep(Duration::from_millis(50));
    // Touch HEAD during the rebase — debounce will fire but
    // dispatcher must classify as DeferredGitOp.
    fs::write(
        repo.join(".git").join("HEAD"),
        "ref: refs/heads/midrebase\n",
    )
    .unwrap();

    // Wait for at least one DeferredGitOp signal.
    let saw_defer = wait_for(Duration::from_millis(DEBOUNCE_MS + 1500), || {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, WatcherEvent::DeferredGitOp))
    });
    let mid_collected = events.lock().unwrap().clone();
    assert!(
        saw_defer,
        "AS-008b: rebase-in-progress must defer reindex; got {mid_collected:?}"
    );
    let early_fires = mid_collected
        .iter()
        .filter(|e| matches!(e, WatcherEvent::ReindexFired))
        .count();
    assert_eq!(
        early_fires, 0,
        "AS-008b: NO ReindexFired while REBASE_HEAD present; got {mid_collected:?}"
    );
}
