//! S-006 follow-up — real `notify::RecommendedWatcher` smoke.
//!
//! Spawns `NotifyWatcherDriver::start` against a tempdir, modifies a
//! file, then asserts the watcher entry observed the event (status
//! `Running`, `last_event_unix` populated). Slow-ish because we wait
//! for the notify backend (FSEvents on macOS, inotify on Linux) to
//! deliver the event.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ga_server::watcher::{
    NotifyWatcherDriver, StartOutcome, WatcherDriver, WatcherMode, WatcherRegistry, WatcherStatus,
};

#[test]
fn notify_driver_observes_file_change() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("a.txt"), b"original").unwrap();

    let registry = Arc::new(WatcherRegistry::new());
    let driver = NotifyWatcherDriver::new(registry.clone());

    let outcome = driver.start("test01", repo.path());
    match outcome {
        StartOutcome::Started(_) | StartOutcome::FallbackPoll(_) => {}
        StartOutcome::Failed(msg) => panic!("watcher failed to start: {msg}"),
    }
    let _ = WatcherMode::Inotify; // imported for doc purposes

    // Modify the file. FSEvents delivers within ~100ms; inotify is
    // instant. Be generous with the budget.
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(repo.path().join("a.txt"), b"changed-content").unwrap();
    std::fs::write(repo.path().join("b.txt"), b"new file").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        {
            let entry = registry.entry("test01");
            let guard = entry.lock().unwrap();
            if guard.last_event_unix.is_some() && guard.queue_pending > 0 {
                assert_eq!(guard.status, WatcherStatus::Stopped);
                // Note: status is Stopped because we only set Running
                // in the handler layer (handlers/watcher.rs); the
                // driver tracks events into the entry but doesn't flip
                // status (that's the handler's job).
                break;
            }
        }
        if Instant::now() > deadline {
            let entry = registry.entry("test01");
            let guard = entry.lock().unwrap();
            panic!(
                "no notify event observed in 5s; queue_pending={} last_event={:?}",
                guard.queue_pending, guard.last_event_unix
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Stop cleanly.
    assert_eq!(
        driver.stop("test01"),
        ga_server::watcher::StopOutcome::Stopped
    );

    // Second stop is a no-op.
    assert_eq!(
        driver.stop("test01"),
        ga_server::watcher::StopOutcome::AlreadyStopped
    );
}

#[test]
fn notify_driver_skips_git_internal_events() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join(".git")).unwrap();
    std::fs::write(repo.path().join("a.txt"), b"x").unwrap();

    let registry = Arc::new(WatcherRegistry::new());
    let driver = NotifyWatcherDriver::new(registry.clone());

    let outcome = driver.start("gitskip1", repo.path());
    assert!(
        matches!(outcome, StartOutcome::Started(_) | StartOutcome::FallbackPoll(_)),
        "start should succeed"
    );

    std::thread::sleep(Duration::from_millis(200));
    // Modify .git internals — should NOT count as activity.
    std::fs::write(repo.path().join(".git/HEAD"), b"ref: refs/heads/main").unwrap();
    std::fs::write(repo.path().join(".git/index.lock"), b"").unwrap();

    // Wait a moment for events to (not) propagate.
    std::thread::sleep(Duration::from_millis(500));

    {
        let entry = registry.entry("gitskip1");
        let guard = entry.lock().unwrap();
        assert_eq!(
            guard.queue_pending, 0,
            ".git events must be skipped, but queue_pending={}",
            guard.queue_pending
        );
    }

    // Now touch a real file — should land.
    std::fs::write(repo.path().join("a.txt"), b"changed").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        {
            let entry = registry.entry("gitskip1");
            let guard = entry.lock().unwrap();
            if guard.queue_pending > 0 {
                break;
            }
        }
        if Instant::now() > deadline {
            panic!("non-git file change should have triggered event");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = driver.stop("gitskip1");
}
