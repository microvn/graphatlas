//! Test helper binary for multi-process lock scenarios — v1.5 PR6.1
//! (multi-mcp) M-3.
//!
//! Acquires a `LockFile` on the given cache layout in the requested mode,
//! prints `READY\n` to stdout so the parent test process knows the lock
//! is held, then holds the lock for `--secs` seconds or until SIGTERM.
//!
//! Usage:
//!   ga_index_lock_holder --cache-root <dir> --repo-root <dir> \
//!                        --hold {exclusive|shared} --secs <N>
//!
//! Sync protocol (matches the integration test harness in
//! `crates/ga-index/tests/multi_process_lock.rs`):
//!   1. Parent spawns the helper via `std::process::Command`.
//!   2. Parent reads one line from helper stdout — `READY` confirms lock
//!      acquired. Any other line is a fatal error.
//!   3. Parent runs its assertions (e.g. calls `Store::open` and expects
//!      attached-read-only outcome).
//!   4. Parent sends SIGTERM (or waits for `--secs` to elapse).

use ga_index::cache::CacheLayout;
use ga_index::lock::LockFile;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cache_root: Option<PathBuf> = None;
    let mut repo_root: Option<PathBuf> = None;
    let mut hold = "exclusive".to_string();
    let mut secs: u64 = 30;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cache-root" => {
                i += 1;
                cache_root = Some(PathBuf::from(&args[i]));
            }
            "--repo-root" => {
                i += 1;
                repo_root = Some(PathBuf::from(&args[i]));
            }
            "--hold" => {
                i += 1;
                hold = args[i].clone();
            }
            "--secs" => {
                i += 1;
                secs = args[i].parse().expect("--secs must be u64");
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let cache_root = cache_root.expect("--cache-root is required");
    let repo_root = repo_root.expect("--repo-root is required");
    let layout = CacheLayout::for_repo(&cache_root, &repo_root);
    layout.ensure_dir().expect("ensure_dir");

    let lock = match hold.as_str() {
        "exclusive" => {
            LockFile::try_acquire_exclusive(&layout, "test-holder-excl").expect("acquire exclusive")
        }
        "shared" => {
            LockFile::try_acquire_shared(&layout, "test-holder-shared").expect("acquire shared")
        }
        other => {
            eprintln!("--hold must be 'exclusive' or 'shared', got {other}");
            std::process::exit(2);
        }
    };

    // Signal parent that the lock is held. Flush so the parent's
    // BufReader::read_line returns immediately.
    println!("READY");
    std::io::stdout().flush().ok();

    // Hold until the deadline. SIGTERM also ends the process.
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }

    drop(lock);
}
