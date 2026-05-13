//! v1.5 PR8 — `.git/`-scoped FS watcher helper tests (triggers S-003).
//!
//! Covers the pure-function pieces of the watcher: state-file probe
//! (AS-008/008b), bench-fixture refusal (AS-010), watch-target selection
//! (AS-007 happy path inputs), and ENOSPC fallback classification
//! (AS-011). The end-to-end notify/RecommendedWatcher event-fire path is
//! covered by `watcher_smoke.rs` (separate file because it needs a real
//! tokio runtime + filesystem events).

use ga_mcp::watcher::{
    is_bench_fixture_path, is_git_op_in_progress, should_fallback_to_polling, validate_repo_root,
    watch_targets, WatcherInitError,
};
use std::fs;
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

// =====================================================================
// AS-007 — watch_targets returns the three expected paths
// =====================================================================

#[test]
fn watch_targets_returns_head_index_and_refs_heads_for_initialized_repo() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let targets = watch_targets(&repo);
    let names: Vec<String> = targets
        .iter()
        .map(|p| {
            p.strip_prefix(repo.join(".git"))
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names.iter().any(|n| n == "HEAD"));
    assert!(names.iter().any(|n| n == "index"));
    assert!(names.iter().any(|n| n.replace('\\', "/") == "refs/heads"));
}

#[test]
fn watch_targets_empty_when_dot_git_missing() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("no-git-here");
    fs::create_dir_all(&repo).unwrap();
    assert!(watch_targets(&repo).is_empty());
}

#[test]
fn watch_targets_skips_missing_paths_in_fresh_clone() {
    // Fresh clone before first commit: HEAD exists, index doesn't,
    // refs/heads is empty dir. Watcher must subscribe only to what's
    // actually present.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("fresh");
    fs::create_dir_all(repo.join(".git").join("refs").join("heads")).unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let targets = watch_targets(&repo);
    assert!(targets.iter().any(|p| p.ends_with("HEAD")));
    assert!(
        !targets.iter().any(|p| p.ends_with("index")),
        "non-existent index must not be in the watch list"
    );
}

// =====================================================================
// AS-008 + AS-008b — git-op-in-progress probe
// =====================================================================

#[test]
fn as_008_no_sentinel_means_no_git_op_in_progress() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    assert!(!is_git_op_in_progress(&repo));
}

#[test]
fn as_008_rebase_head_sentinel_flags_op_in_progress() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    fs::write(repo.join(".git").join("REBASE_HEAD"), "abc1234\n").unwrap();
    assert!(is_git_op_in_progress(&repo));
}

#[test]
fn as_008_merge_head_sentinel_flags_op_in_progress() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    fs::write(repo.join(".git").join("MERGE_HEAD"), "abc1234\n").unwrap();
    assert!(is_git_op_in_progress(&repo));
}

#[test]
fn as_008_cherry_pick_head_sentinel_flags_op_in_progress() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    fs::write(repo.join(".git").join("CHERRY_PICK_HEAD"), "abc1234\n").unwrap();
    assert!(is_git_op_in_progress(&repo));
}

#[test]
fn as_008_bisect_log_sentinel_flags_op_in_progress() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    fs::write(repo.join(".git").join("BISECT_LOG"), "git bisect start\n").unwrap();
    assert!(is_git_op_in_progress(&repo));
}

#[test]
fn as_008b_rebase_merge_directory_flags_op_in_progress() {
    // Interactive rebase with conflict leaves a rebase-merge/ directory.
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    fs::create_dir_all(repo.join(".git").join("rebase-merge")).unwrap();
    assert!(is_git_op_in_progress(&repo));
}

#[test]
fn as_008_non_git_path_is_safe_no_op() {
    let tmp = TempDir::new().unwrap();
    let not_a_repo = tmp.path().join("notes");
    fs::create_dir_all(&not_a_repo).unwrap();
    assert!(!is_git_op_in_progress(&not_a_repo));
}

// =====================================================================
// AS-010 — bench fixture refusal
// =====================================================================

#[test]
fn as_010_bench_fixture_path_detected_by_substring() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches").join("fixtures").join("django");
    fs::create_dir_all(&fixture).unwrap();
    assert!(is_bench_fixture_path(&fixture));
}

#[test]
fn as_010_regular_path_is_not_a_bench_fixture() {
    let tmp = TempDir::new().unwrap();
    let regular = tmp.path().join("workspace").join("my-project");
    fs::create_dir_all(&regular).unwrap();
    assert!(!is_bench_fixture_path(&regular));
}

#[test]
fn as_010_validate_repo_root_refuses_bench_fixture() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches").join("fixtures").join("django");
    fs::create_dir_all(fixture.join(".git")).unwrap();
    match validate_repo_root(&fixture) {
        Err(WatcherInitError::BenchFixtureRefused { .. }) => {}
        other => panic!("AS-010: expected BenchFixtureRefused, got {other:?}"),
    }
}

#[test]
fn validate_repo_root_returns_not_a_git_repo_when_dot_git_missing() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("plain-dir");
    fs::create_dir_all(&dir).unwrap();
    match validate_repo_root(&dir) {
        Err(WatcherInitError::NotAGitRepo { .. }) => {}
        other => panic!("expected NotAGitRepo, got {other:?}"),
    }
}

#[test]
fn validate_repo_root_accepts_normal_git_repo() {
    let tmp = TempDir::new().unwrap();
    let repo = init_repo(&tmp);
    let git_dir = validate_repo_root(&repo).expect("normal git repo must validate");
    assert_eq!(git_dir, repo.join(".git"));
}

// =====================================================================
// AS-011 — inotify ENOSPC → polling fallback classification
// =====================================================================

#[test]
fn as_011_enospc_string_triggers_polling_fallback() {
    assert!(should_fallback_to_polling("inotify_add_watch: ENOSPC"));
    assert!(should_fallback_to_polling("No space left on device"));
    assert!(should_fallback_to_polling(
        "fs.inotify.max_user_watches limit reached"
    ));
    assert!(should_fallback_to_polling("Too many open files"));
}

#[test]
fn as_011_unrelated_error_does_not_trigger_polling_fallback() {
    assert!(!should_fallback_to_polling("permission denied"));
    assert!(!should_fallback_to_polling("network is unreachable"));
    assert!(!should_fallback_to_polling(""));
}

#[test]
fn as_011_polling_fallback_classifier_is_case_insensitive() {
    assert!(should_fallback_to_polling("ENOSPC"));
    assert!(should_fallback_to_polling("enospc"));
    assert!(should_fallback_to_polling("Max_User_Watches limit reached"));
}
