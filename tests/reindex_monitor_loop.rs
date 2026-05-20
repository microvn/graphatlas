//! S-005 follow-up — real subprocess + monitor thread exercise.
//!
//! Spawns `graphatlas reindex <tmpdir>` via `SubprocessLauncher`, then
//! polls the shared `JobState` until the monitor thread observes child
//! exit. Asserts state transitions Running → Done with non-zero
//! `duration_ms`.

use std::sync::{Arc, Mutex};

use ga_server::jobs::{JobLauncher, JobState, JobStatus, SubprocessLauncher};
use ga_server::recovery::find_cache_dir;
use tempfile::TempDir;

fn fixture_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        b"pub fn hello() -> &'static str { \"hi\" }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    tmp
}

#[test]
fn monitor_thread_flips_state_to_done_after_subprocess_exits() {
    let cache = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("GRAPHATLAS_CACHE_DIR", cache.path());

    let repo = fixture_repo();
    let repo_path = repo.path().canonicalize().unwrap();

    // Resolve SubprocessLauncher from the test binary's neighbor —
    // CARGO_BIN_EXE_graphatlas points at the binary cargo built for
    // integration tests.
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_graphatlas"));
    let launcher = SubprocessLauncher {
        graphatlas_bin: bin,
    };

    let state = Arc::new(Mutex::new(JobState::new_running()));
    let _pid = launcher
        .spawn_index(&repo_path, false, state.clone())
        .expect("spawn");

    // Poll for terminal state. Budget 30s — reindex of a 1-file
    // fixture is ~2s in dev mode; 30s headroom covers cold-cache lbug init.
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(30);
    loop {
        {
            let st = state.lock().unwrap();
            if matches!(st.status, JobStatus::Done | JobStatus::Error) {
                break;
            }
        }
        if std::time::Instant::now() > deadline {
            let st = state.lock().unwrap();
            panic!(
                "monitor thread didn't transition state in 30s; status={:?} error={:?}",
                st.status, st.error
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let snap = state.lock().unwrap().clone();
    assert_eq!(
        snap.status,
        JobStatus::Done,
        "expected Done after successful reindex; got {:?}, error={:?}",
        snap.status,
        snap.error
    );
    assert!(snap.duration_ms > 0, "duration_ms must be populated");
    assert_eq!(snap.percent, 100.0);

    // Cache must have been written.
    assert!(
        find_cache_dir(cache.path(), "").is_some() || {
            std::fs::read_dir(cache.path())
                .unwrap()
                .flatten()
                .any(|e| e.path().is_dir())
        }
    );

    std::env::remove_var("GRAPHATLAS_CACHE_DIR");
}

#[test]
fn monitor_thread_flips_state_to_error_when_subprocess_fails() {
    // Spawn the launcher against a non-existent path so the subprocess
    // exits non-zero. Verify the monitor thread surfaces it as Error.
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_graphatlas"));
    let launcher = SubprocessLauncher {
        graphatlas_bin: bin,
    };
    let state = Arc::new(Mutex::new(JobState::new_running()));
    let bogus = std::path::PathBuf::from("/tmp/__ga_monitor_bogus_path_xyz__");

    // Even if spawn somehow succeeds (Command::spawn only fails if the
    // binary can't be found / fork fails), reindex against missing path
    // must exit non-zero.
    let _ = launcher.spawn_index(&bogus, false, state.clone());

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        {
            let st = state.lock().unwrap();
            if matches!(st.status, JobStatus::Done | JobStatus::Error) {
                break;
            }
        }
        if std::time::Instant::now() > deadline {
            let st = state.lock().unwrap();
            panic!("no transition in 30s; status={:?}", st.status);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let snap = state.lock().unwrap().clone();
    assert_eq!(
        snap.status,
        JobStatus::Error,
        "bogus path reindex must surface as Error; got {:?}",
        snap.status
    );
    assert!(snap.error.is_some());
    assert!(snap.duration_ms > 0);
}
