//! M3 scoring loop for `architecture` UC — compares `ga_architecture`'s
//! edges against the Ha-import-edge GT.
//!
//! Spec target (Verification §): edge correlation (F1 fallback) ≥ 0.6.

use ga_bench::m3_runner::{score_architecture, ScoreOpts, SpecStatus};
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn opts(tmp: &TempDir) -> ScoreOpts {
    ScoreOpts {
        fixture_name: "synth-arch".to_string(),
        fixture_dir: tmp.path().join("repo"),
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["ga".to_string()],
        gt_path: None,
        split: None,
    }
}

#[test]
fn architecture_emits_row_with_edge_f1() {
    // Two-module fixture: a/ imports from b/.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("b/__init__.py"), "");
    write(
        &repo.join("a/x.py"),
        "from b.foo import F\n\ndef use():\n    return F\n",
    );
    write(&repo.join("b/foo.py"), "F = 1\n");

    let rows = score_architecture(&opts(&tmp)).expect("scoring must succeed");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.uc, "architecture");
    assert!(
        (row.spec_target - 0.6).abs() < 1e-9,
        "architecture spec_target locked at 0.6 per spec Verification §"
    );
    assert!(
        row.secondary_metrics.contains_key("edge_f1")
            || row.secondary_metrics.contains_key("expected_edge_count"),
        "secondary_metrics should include edge-related diagnostics; got: {:?}",
        row.secondary_metrics.keys().collect::<Vec<_>>()
    );
}

#[test]
fn architecture_empty_fixture_returns_no_row() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("repo")).unwrap();
    let rows = score_architecture(&opts(&tmp)).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn architecture_non_ga_retriever_emits_deferred_row() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("b/__init__.py"), "");
    write(&repo.join("a/x.py"), "from b import g\n");
    write(&repo.join("b/__init__.py"), "g = 1\n");
    let mut o = opts(&tmp);
    o.retrievers = vec!["cgc".to_string()];
    let rows = score_architecture(&o).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].spec_status, SpecStatus::Deferred);
}
