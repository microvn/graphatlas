//! M3 scoring loop for `risk` UC — runs `ga_risk` per file in fixture
//! and scores against Hr-text GT.

use ga_bench::m3_runner::{score_risk, ScoreOpts, SpecStatus};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

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

fn opts(repo_dir: &Path, cache_dir: &Path) -> ScoreOpts {
    ScoreOpts {
        fixture_name: "synth-risk".to_string(),
        fixture_dir: repo_dir.to_path_buf(),
        cache_root: cache_dir.to_path_buf(),
        retrievers: vec!["ga".to_string()],
        gt_path: None,
        split: None,
    }
}

#[test]
fn risk_emits_row_with_f1_and_mae_secondary() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    fs::write(repo.join("a.py"), "def a():\n    return 1\n").unwrap();
    fs::write(repo.join("b.py"), "def b():\n    return 1\n").unwrap();
    git(&repo, &["add", "a.py", "b.py"]);
    git(&repo, &["commit", "-m", "feat: initial"]);
    fs::write(repo.join("a.py"), "def a():\n    return 99  # patched\n").unwrap();
    git(&repo, &["add", "a.py"]);
    git(&repo, &["commit", "-m", "fix: a was wrong"]);

    let cache = tmp.path().join("cache");
    let rows = score_risk(&opts(&repo, &cache)).expect("scoring must succeed");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.uc, "risk");
    assert_eq!(row.retriever, "ga");
    assert!(
        (row.spec_target - 0.80).abs() < 1e-9,
        "risk spec_target locked at 0.80 per Verification §"
    );
    // 2026-05-02 methodology fix — primary score is max-F1 over PR curve.
    // Secondary still surfaces F1@0.30 cutoff for backwards-compat
    // interpretability.
    assert!(
        row.secondary_metrics
            .contains_key("precision_at_0.30_cutoff"),
        "secondary_metrics must include cutoff precision; got: {:?}",
        row.secondary_metrics.keys().collect::<Vec<_>>()
    );
    assert!(row.secondary_metrics.contains_key("recall_at_0.30_cutoff"));
    assert!(row.secondary_metrics.contains_key("f1_at_0.30_cutoff"));
    assert!(row.secondary_metrics.contains_key("pr_at_0.30_precision"));
    assert!(row.secondary_metrics.contains_key("pr_at_0.30_recall"));
    assert!(row.secondary_metrics.contains_key("pr_at_0.30_f1"));
    assert!(
        row.secondary_metrics.contains_key("max_f1_threshold"),
        "primary score is max-F1 over PR curve — must surface winning threshold"
    );
}

#[test]
fn risk_empty_fixture_returns_no_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let cache = tmp.path().join("cache");
    let rows = score_risk(&opts(&repo, &cache)).unwrap();
    assert!(rows.is_empty(), "non-git fixture → no rows; got {:?}", rows);
}

#[test]
fn risk_non_ga_retriever_emits_deferred_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    fs::write(repo.join("a.py"), "def a():\n    return 1\n").unwrap();
    git(&repo, &["add", "a.py"]);
    git(&repo, &["commit", "-m", "fix: a"]);

    let cache = tmp.path().join("cache");
    let mut o = opts(&repo, &cache);
    o.retrievers = vec!["cgc".to_string()];
    let rows = score_risk(&o).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].spec_status, SpecStatus::Deferred);
}
