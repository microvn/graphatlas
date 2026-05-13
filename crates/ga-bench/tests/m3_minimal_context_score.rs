//! End-to-end scoring loop for the `minimal_context` UC.
//!
//! Migrated 2026-04-28 from `Hmc-budget` (tasks-v6) to `Hmc-gitmine`
//! (ground-truth.json). Schema differences vs. the old test helper:
//! - top-level `tasks` array (not JSONL)
//! - field names: `task_id`/`seed_symbol`/`seed_file`/`expected_files`
//! - per-task `split: "test"` (rule defaults to scoring split=test only)
//! - `base_commit` blank ⇒ pinning silently skipped (synthetic fixtures
//!   are not git repos).

use ga_bench::m3_runner::{score_minimal_context, ScoreOpts, SpecStatus};
use std::fs;
use tempfile::TempDir;

fn synthetic_repo_with_task(repo_name: &str, target_symbol: &str, expected: &[&str]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    fs::write(
        repo.join("util.py"),
        format!("def {target_symbol}(x):\n    return x + 1\n"),
    )
    .unwrap();
    fs::write(
        repo.join("driver.py"),
        format!(
            "from util import {target_symbol}\n\ndef driver():\n    return {target_symbol}(1)\n"
        ),
    )
    .unwrap();

    let task = serde_json::json!({
        "task_id": format!("{repo_name}-synthetic"),
        "repo": repo_name,
        "lang": "python",
        "base_commit": "",
        "fix_commit": "",
        "subject": "synthetic test fixture",
        "seed_file": "util.py",
        "seed_symbol": target_symbol,
        "source_files": ["util.py"],
        "expected_files": expected,
        "expected_tests": [],
        "should_touch_files": [],
        "max_expected_depth": 1,
        "split": "test",
    });
    let gt = serde_json::json!({
        "schema_version": 3,
        "source": "synthetic",
        "uc": "impact",
        "spec": "test",
        "mining_tool": "synthetic",
        "total_tasks": 1,
        "per_repo": {repo_name: 1},
        "per_lang": {"python": 1},
        "splits": {"dev": 0, "test": 1},
        "tasks": [task],
    });

    let datasets = tmp.path().join("benches/uc-impact");
    fs::create_dir_all(&datasets).unwrap();
    fs::write(datasets.join("ground-truth.json"), gt.to_string()).unwrap();
    tmp
}

fn opts(tmp: &TempDir, fixture_name: &str) -> ScoreOpts {
    ScoreOpts {
        fixture_name: fixture_name.to_string(),
        fixture_dir: tmp.path().join("repo"),
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["ga".to_string()],
        gt_path: Some(
            tmp.path()
                .join("benches/uc-impact")
                .join("ground-truth.json"),
        ),
        split: None,
    }
}

#[test]
fn score_minimal_context_emits_row_with_file_recall_and_precision() {
    let tmp = synthetic_repo_with_task("synth-pass", "myfunc", &["util.py", "driver.py"]);
    let rows = score_minimal_context(&opts(&tmp, "synth-pass")).expect("scoring must succeed");
    assert_eq!(rows.len(), 1, "one retriever (ga) → one row");
    let row = &rows[0];
    assert_eq!(row.retriever, "ga");
    assert_eq!(row.fixture, "synth-pass");
    assert_eq!(row.uc, "minimal_context");
    assert!(
        row.score >= 0.5,
        "ga should hit ≥0.5 file_recall on a 2-file expected set with util.py + driver.py; got {}",
        row.score
    );
    let keys: Vec<&str> = row.secondary_metrics.keys().map(String::as_str).collect();
    for required in &[
        "file_precision",
        "test_recall",
        "recall_per_1k_tokens",
        "truncation_correctness_rate",
        "seed_symbol_not_found_count",
        "pin_failed_count",
        "pin_enabled",
        "task_count",
    ] {
        assert!(
            keys.contains(required),
            "secondary_metrics missing `{required}`; have {keys:?}"
        );
    }
}

#[test]
fn high_recall_yields_pass_status() {
    let tmp = synthetic_repo_with_task("synth-pass2", "winning_func", &["util.py"]);
    let rows = score_minimal_context(&opts(&tmp, "synth-pass2")).unwrap();
    let row = &rows[0];
    assert_eq!(
        row.spec_status,
        SpecStatus::Pass,
        "score {} ≥ 0.70 must yield PASS; row: {:?}",
        row.score,
        row
    );
    assert!(
        (row.spec_target - 0.70).abs() < 1e-9,
        "spec_target for minimal_context locked at 0.70; got {}",
        row.spec_target
    );
}

#[test]
fn low_recall_yields_fail_status() {
    let tmp = synthetic_repo_with_task(
        "synth-fail",
        "lonely",
        &["nonexistent/a.py", "nonexistent/b.py", "nonexistent/c.py"],
    );
    let rows = score_minimal_context(&opts(&tmp, "synth-fail")).unwrap();
    let row = &rows[0];
    assert!(
        row.score < 0.70,
        "expected low recall when expected_files are unreachable; got {}",
        row.score
    );
    assert_eq!(row.spec_status, SpecStatus::Fail);
}

#[test]
fn returns_empty_for_unknown_fixture() {
    let tmp = synthetic_repo_with_task("synth-empty-host", "foo", &["util.py"]);
    let mut o = opts(&tmp, "no-such-repo-in-tasks");
    o.retrievers = vec!["ga".to_string()];
    let rows = score_minimal_context(&o).unwrap();
    assert!(
        rows.is_empty(),
        "no tasks for fixture ⇒ no rows; got {} rows",
        rows.len()
    );
}
