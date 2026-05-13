//! Tools S-006 cluster C11 follow-up — uc-impact bench smoke.
//!
//! Drives the ga retriever against the consolidated M2 ground-truth
//! (`benches/uc-impact/ground-truth.json`) filtered to repo=gin, split=Dev.
//! Verifies the bench plumbing works end-to-end. Quality measurement
//! (precision/recall/F1) is the bench runner's job — this test only pins
//! that the retriever produces relevant output and doesn't silently regress
//! to an empty result.

use ga_bench::retriever::Retriever;
use ga_bench::retrievers::GaRetriever;
use ga_bench::{M2GroundTruth, M2Task, Split};
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn uc_impact_gin_smoke_runs_and_scores() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixture = root.join("benches/fixtures/gin");

    if !gt_path.exists() {
        eprintln!("[SKIP] GT missing: {}", gt_path.display());
        return;
    }
    if !fixture.join("auth.go").exists() {
        eprintln!(
            "[SKIP] fixture submodule not checked out: {}",
            fixture.display()
        );
        return;
    }

    let gt = M2GroundTruth::load(&gt_path).expect("M2 GT load");
    assert_eq!(gt.uc, "impact");

    let dev_gin: Vec<&M2Task> = gt
        .tasks
        .iter()
        .filter(|t| t.repo == "gin" && t.split == Split::Dev)
        .collect();
    assert!(
        !dev_gin.is_empty(),
        "expected at least one gin/dev task in {}",
        gt_path.display()
    );

    let tmp = tempfile::TempDir::new().unwrap();
    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    ret.setup(&fixture).expect("ga setup on gin fixture");

    // Smoke is "pipeline runs end-to-end and at least one dev task overlaps
    // GT". Per-task `actual.is_empty()` is acceptable: the fixture submodule
    // is at HEAD while each task pins a `base_commit` from history, so a seed
    // symbol may legitimately not exist at HEAD (renamed/removed). The
    // composite test (uc_impact_composite.rs) is the actual measurement.
    let mut total_with_results = 0;
    let mut total_with_overlap = 0;

    for task in &dev_gin {
        let q = serde_json::json!({
            "symbol": task.seed_symbol,
            "file":   task.seed_file,
        });
        let actual = ret.query("impact", &q).expect("impact query runs");
        eprintln!(
            "task={} seed={} actual_len={}",
            task.task_id,
            task.seed_symbol,
            actual.len()
        );
        if actual.is_empty() {
            continue;
        }
        total_with_results += 1;

        let prod: std::collections::HashSet<&String> = task.expected_files.iter().collect();
        let tests: std::collections::HashSet<&String> = task.expected_tests.iter().collect();
        if actual.iter().any(|p| prod.contains(p) || tests.contains(p)) {
            total_with_overlap += 1;
        }
    }

    assert!(
        total_with_results >= 1,
        "no dev/gin task returned any results — pipeline broken"
    );
    assert!(
        total_with_overlap >= 1,
        "no dev/gin task overlapped GT (seen {total_with_results} non-empty results) \
         — retriever produces output but never relevant"
    );
}
