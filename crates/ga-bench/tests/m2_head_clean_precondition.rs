//! v1.2-php S-002 AS-020 — fixture workspace HEAD-clean precondition.
//!
//! Per `project_m3_submodule_drift` memory: full ga-bench test suite leaves
//! fixture submodules dirty after an M3 HmcGitmine run. M2 runner inheriting
//! the dirty state mines biased GT against the wrong file contents.
//!
//! `assert_workspace_clean` rejects dirty trees loudly so the bias never
//! enters published numbers.

use ga_bench::fixture_workspace::assert_workspace_clean;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn init_clean_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path();
    let run = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap()
    };
    run(&["init", "--quiet"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(path.join("README.md"), "test\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial", "--quiet"]);
    tmp
}

#[test]
fn clean_workspace_returns_ok() {
    let tmp = init_clean_repo();
    let result = assert_workspace_clean(tmp.path());
    assert!(result.is_ok(), "clean workspace must return Ok: {result:?}");
}

#[test]
fn dirty_workspace_returns_fixture_corrupted() {
    let tmp = init_clean_repo();
    // Simulate M3-drift contamination: modify a tracked file.
    fs::write(tmp.path().join("README.md"), "modified\n").unwrap();

    let result = assert_workspace_clean(tmp.path());
    assert!(
        result.is_err(),
        "dirty workspace MUST return Err: {result:?}"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("FixtureCorrupted"),
        "error message must include 'FixtureCorrupted': {msg}"
    );
    assert!(
        msg.contains("uncommitted") || msg.contains("M3-drift"),
        "error message must mention dirty / M3-drift cause: {msg}"
    );
}

#[test]
fn dirty_workspace_untracked_file_returns_fixture_corrupted() {
    // Untracked files also count as dirty per git status --porcelain — they
    // could be artifacts from a half-completed M3 mining step.
    let tmp = init_clean_repo();
    fs::write(tmp.path().join("untracked.txt"), "stray\n").unwrap();

    let result = assert_workspace_clean(tmp.path());
    assert!(
        result.is_err(),
        "untracked file MUST trigger FixtureCorrupted: {result:?}"
    );
}

#[test]
fn non_git_dir_returns_fixture_corrupted() {
    let tmp = TempDir::new().unwrap();
    let result = assert_workspace_clean(tmp.path());
    assert!(result.is_err(), "non-git dir MUST return Err");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("not a git repo") || msg.contains("FixtureCorrupted"),
        "non-git error message: {msg}"
    );
}

#[test]
fn cleaned_after_reset_returns_ok() {
    let tmp = init_clean_repo();
    // Dirty the tree.
    fs::write(tmp.path().join("README.md"), "modified\n").unwrap();
    assert!(assert_workspace_clean(tmp.path()).is_err());

    // git reset --hard restores clean state.
    Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["reset", "--hard", "--quiet"])
        .output()
        .unwrap();

    let result = assert_workspace_clean(tmp.path());
    assert!(
        result.is_ok(),
        "after `git reset --hard`, workspace MUST be clean again: {result:?}"
    );
}
