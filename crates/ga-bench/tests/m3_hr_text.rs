//! Hr-text rule tests — exercises real git in a tempdir to validate
//! anchor-based time-window mining.

use ga_bench::gt_gen::hr_text::HrText;
use ga_bench::gt_gen::{GeneratedTask, GtRule};
use ga_index::Store;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn empty_store(repo: &Path) -> (Store, TempDir) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache, repo).unwrap();
    (store, tmp)
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@local")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@local")
        .status()
        .expect("git available");
    assert!(status.success(), "git {:?} failed", args);
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn task_for<'a>(tasks: &'a [GeneratedTask], file: &str) -> &'a GeneratedTask {
    tasks
        .iter()
        .find(|t| t.query.get("file").and_then(|v| v.as_str()) == Some(file))
        .unwrap_or_else(|| panic!("no Hr-text task for {file}"))
}

/// Build a tiny git repo with two commits: `risky.py` modified by a
/// "fix bug" commit, `safe.py` modified by a "feat" commit.
fn synthetic_git_repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-b", "main"]);
    write(&repo.join("risky.py"), "def f():\n    return 1\n");
    write(&repo.join("safe.py"), "def g():\n    return 2\n");
    git(&repo, &["add", "risky.py", "safe.py"]);
    git(&repo, &["commit", "-m", "feat: initial"]);
    // Second commit on risky.py with bug-keyword subject.
    write(
        &repo.join("risky.py"),
        "def f():\n    return 99  # corrected\n",
    );
    git(&repo, &["add", "risky.py"]);
    git(&repo, &["commit", "-m", "fix: off-by-one bug in f"]);
    // Third commit on safe.py with non-bug subject.
    write(&repo.join("safe.py"), "def g():\n    return 22\n");
    git(&repo, &["add", "safe.py"]);
    git(&repo, &["commit", "-m", "feat: refactor g"]);
    tmp
}

#[test]
fn hr_text_marks_risky_when_bug_keyword_in_history() {
    let tmp = synthetic_git_repo();
    let repo = tmp.path();
    let (store, _t) = empty_store(repo);
    let tasks = HrText.scan(&store, repo).unwrap();
    let risky = task_for(&tasks, "risky.py");
    assert_eq!(
        risky.query.get("expected_risky").and_then(|v| v.as_bool()),
        Some(true),
        "risky.py has a `fix:` commit → expected_risky=true; got: {}",
        risky.query
    );
    assert!(
        risky
            .query
            .get("bug_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 1,
        "bug_count must be ≥1 when `fix:` matches; got: {}",
        risky.query
    );
}

#[test]
fn hr_text_does_not_mark_risky_when_no_bug_keyword() {
    let tmp = synthetic_git_repo();
    let repo = tmp.path();
    let (store, _t) = empty_store(repo);
    let tasks = HrText.scan(&store, repo).unwrap();
    let safe = task_for(&tasks, "safe.py");
    assert_eq!(
        safe.query.get("expected_risky").and_then(|v| v.as_bool()),
        Some(false),
        "safe.py has only `feat:` commits → expected_risky=false; got: {}",
        safe.query
    );
}

#[test]
fn hr_text_records_anchor_and_window_for_audit() {
    let tmp = synthetic_git_repo();
    let repo = tmp.path();
    let (store, _t) = empty_store(repo);
    let tasks = HrText.scan(&store, repo).unwrap();
    let any = tasks.first().expect("at least one task");
    assert!(
        any.query
            .get("anchor")
            .and_then(|v| v.as_str())
            .map(|s| s.len())
            == Some(40),
        "anchor must be a 40-char SHA from git rev-parse HEAD; got: {}",
        any.query
    );
    assert_eq!(
        any.query.get("window_days").and_then(|v| v.as_u64()),
        Some(90),
        "window_days locked at 90 per AS-001"
    );
}

#[test]
fn hr_text_id_uc_match_spec() {
    let r = HrText;
    assert_eq!(r.id(), "Hr-text");
    assert_eq!(r.uc(), "risk");
}

#[test]
fn hr_text_non_git_fixture_returns_empty_no_panic() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    fs::write(repo.join("foo.py"), "def f(): return 1\n").unwrap();
    let (store, _t) = empty_store(repo);
    let tasks = HrText.scan(&store, repo).unwrap();
    assert!(
        tasks.is_empty(),
        "non-git fixture → empty Vec; got {} tasks",
        tasks.len()
    );
}
