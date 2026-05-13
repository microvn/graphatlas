//! M3 scoring loop for `dead_code` UC — runs `ga_dead_code` against
//! Hd-ast GT and emits `M3LeaderboardRow` with PASS/FAIL spec_status.
//!
//! Spec target (Verification §): Precision ≥ 0.85.

use ga_bench::m3_runner::{score_dead_code, ScoreOpts, SpecStatus};
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn opts(tmp: &TempDir) -> ScoreOpts {
    ScoreOpts {
        fixture_name: "synth".to_string(),
        fixture_dir: tmp.path().join("repo"),
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["ga".to_string()],
        gt_path: None,
        split: None,
    }
}

#[test]
fn dead_code_pass_when_ga_finds_only_real_dead_symbols() {
    // Repo with one real dead helper + one live helper. ga_dead_code
    // should return exactly the dead helper → precision 1.0 ≥ 0.85 → PASS.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(
        &repo.join("util.py"),
        "def dead_one():\n    return 1\n\ndef live_one():\n    return 2\n",
    );
    write(
        &repo.join("driver.py"),
        "from util import live_one\n\ndef driver():\n    return live_one()\n",
    );

    let rows = score_dead_code(&opts(&tmp)).expect("scoring must succeed");
    assert_eq!(rows.len(), 1, "one retriever → one row");
    let row = &rows[0];
    assert_eq!(row.uc, "dead_code");
    assert_eq!(row.retriever, "ga");
    assert!(
        (row.spec_target - 0.85).abs() < 1e-9,
        "dead_code spec_target locked at 0.85; got {}",
        row.spec_target
    );
    assert_eq!(
        row.spec_status,
        SpecStatus::Pass,
        "ga should find dead_one and only dead_one → precision ≥ 0.85; got score={} status={:?}",
        row.score,
        row.spec_status
    );
    assert!(
        row.secondary_metrics.contains_key("recall"),
        "secondary_metrics must include recall; keys: {:?}",
        row.secondary_metrics.keys().collect::<Vec<_>>()
    );
    assert!(row.secondary_metrics.contains_key("f1"));
}

#[test]
fn dead_code_empty_fixture_returns_no_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let rows = score_dead_code(&opts(&tmp)).unwrap();
    // Empty fixture ⇒ no GT tasks; convention from minimal_context: no
    // 0/0 sentinel row.
    assert!(
        rows.is_empty(),
        "empty fixture → empty rows; got {:?}",
        rows
    );
}

#[test]
fn dead_code_non_ga_retriever_emits_deferred_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("u.py"), "def f():\n    return 1\n");
    let mut o = opts(&tmp);
    o.retrievers = vec!["cgc".to_string()];
    let rows = score_dead_code(&o).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].spec_status,
        SpecStatus::Deferred,
        "competitor adapters land Phase 4; non-`ga` retrievers get DEFERRED rows"
    );
}
