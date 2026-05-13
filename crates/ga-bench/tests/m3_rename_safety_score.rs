//! M3 scoring loop for `rename_safety` UC — runs `ga_rename_safety` per
//! Hrn-static GT target and emits one M3LeaderboardRow per retriever.
//!
//! Spec target (Verification §): Recall ≥ 0.90 trên `targets_unique` tier;
//! ≥ 0.70 trên `targets_polymorphic` tier with file_hint.

use ga_bench::m3_runner::{score_rename_safety, ScoreOpts, SpecStatus};
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn opts(tmp: &TempDir) -> ScoreOpts {
    ScoreOpts {
        fixture_name: "synth-rn".to_string(),
        fixture_dir: tmp.path().join("repo"),
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["ga".to_string()],
        gt_path: None,
        split: None,
    }
}

#[test]
fn rename_safety_pass_when_ga_finds_all_unique_target_sites() {
    // util::renameable defined once + called once. Recall = 1.0 ≥ 0.90 → PASS.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("util.py"), "def renameable():\n    return 1\n");
    write(
        &repo.join("d.py"),
        "from util import renameable\n\ndef driver():\n    return renameable()\n",
    );
    let rows = score_rename_safety(&opts(&tmp)).expect("scoring must succeed");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.uc, "rename_safety");
    assert!(
        row.spec_status == SpecStatus::Pass,
        "unique-tier recall ≥ 0.90 must yield PASS; got score={} status={:?}",
        row.score,
        row.spec_status
    );
    assert!(
        row.secondary_metrics.contains_key("recall_unique"),
        "secondary_metrics must include per-tier recall; got: {:?}",
        row.secondary_metrics.keys().collect::<Vec<_>>()
    );
    assert!(row.secondary_metrics.contains_key("recall_polymorphic"));
}

#[test]
fn rename_safety_empty_fixture_returns_no_row() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("repo")).unwrap();
    let rows = score_rename_safety(&opts(&tmp)).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn rename_safety_non_ga_retriever_emits_deferred_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("u.py"), "def f():\n    return 1\n");
    let mut o = opts(&tmp);
    o.retrievers = vec!["cgc".to_string()];
    let rows = score_rename_safety(&o).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].spec_status, SpecStatus::Deferred);
}
