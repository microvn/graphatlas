//! Tools S-006 cluster C11 follow-up #20 — honest 4-dim ImpactScore
//! measurement on the consolidated M2 ground-truth, filtered to dev split
//! per-fixture. Reports actual per-dim scores + composite per fixture.
//!
//! No "0.80 pass/fail" assertion here — the per-fixture rows are
//! measurement-only. A weighted cross-fixture aggregate is reported at the
//! end of `uc_impact_weighted_avg_across_fixtures` so M2 gate progress is
//! visible as more fixtures land.

use ga_bench::retriever::Retriever;
use ga_bench::retrievers::GaRetriever;
use ga_bench::score::impact_score;
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

/// Per-fixture driver. Returns `(composite_avg, task_count)` so the
/// weighted-average test can aggregate without re-running. Skips (returns
/// `None`) when the fixture submodule is missing or the GT has no
/// matching dev tasks.
fn run_fixture(name: &str, sentinel_relpath: &str, gt: &M2GroundTruth) -> Option<(f64, usize)> {
    let root = workspace_root();
    let fixture = root.join(format!("benches/fixtures/{name}"));

    if !fixture.join(sentinel_relpath).exists() {
        eprintln!(
            "[SKIP {name}] fixture submodule not checked out (missing {sentinel_relpath}): {}",
            fixture.display()
        );
        return None;
    }

    let dev_tasks: Vec<&M2Task> = gt
        .tasks
        .iter()
        .filter(|t| t.repo == name && t.split == Split::Dev)
        .collect();
    if dev_tasks.is_empty() {
        eprintln!("[SKIP {name}] no dev tasks for repo={name} in ground-truth.json");
        return None;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let mut ret = GaRetriever::new(tmp.path().join(".cache"));
    if let Err(e) = ret.setup(&fixture) {
        eprintln!(
            "[SKIP {name}] indexer setup failed — fixture unsupported by current engine: {e}"
        );
        return None;
    }

    let mut composites = Vec::new();
    let mut test_recalls = Vec::new();
    let mut completenesses = Vec::new();
    let mut precisions = Vec::new();
    let mut depth_f1s = Vec::new();

    println!("\n=== uc-impact {name} composite measurement (dev split) ===");
    println!("Target per AS-012: composite ≥ 0.80");
    println!("Formula: 0.4·test_recall + 0.3·completeness + 0.15·depth_f1 + 0.15·precision\n");

    for task in &dev_tasks {
        let q = serde_json::json!({
            "symbol": task.seed_symbol,
            "file":   task.seed_file,
        });
        let actual = ret
            .query_impact(&q)
            .expect("query_impact supported")
            .expect("query runs");

        let score = impact_score(
            &actual.files,
            &actual.tests,
            Some(actual.max_depth),
            &task.expected_files,
            &task.expected_tests,
            task.max_expected_depth,
            &task.should_touch_files,
        );

        println!("[{}]", task.task_id);
        println!(
            "  actual: files={} tests={} routes={} depth={}",
            actual.files.len(),
            actual.tests.len(),
            actual.routes.len(),
            actual.max_depth
        );
        println!(
            "  expected: files={} tests={} depth={:?}",
            task.expected_files.len(),
            task.expected_tests.len(),
            task.max_expected_depth
        );
        println!(
            "  dims: test_recall={:.3} completeness={:.3} depth_f1={:.3} precision={:.3}",
            score.test_recall, score.completeness, score.depth_f1, score.precision
        );
        println!("  composite={:.3}\n", score.composite);

        composites.push(score.composite);
        test_recalls.push(score.test_recall);
        completenesses.push(score.completeness);
        precisions.push(score.precision);
        depth_f1s.push(score.depth_f1);
    }

    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let avg_composite = avg(&composites);

    println!("=== AGGREGATE {name} ({} tasks) ===", dev_tasks.len());
    println!("avg composite: {avg_composite:.3}");
    println!("avg test_recall: {:.3}", avg(&test_recalls));
    println!("avg completeness: {:.3}", avg(&completenesses));
    println!("avg depth_f1: {:.3}", avg(&depth_f1s));
    println!("avg precision: {:.3}", avg(&precisions));
    if avg_composite >= 0.80 {
        println!("✅ MEETS target ≥0.80");
    } else {
        println!("⚠️  GAP to 0.80 target: {:.3}", 0.80 - avg_composite);
    }

    for c in &composites {
        assert!(
            *c >= 0.0 && *c <= 1.0,
            "composite must be in [0,1]: got {c}"
        );
    }

    Some((avg_composite, dev_tasks.len()))
}

/// Fixture registry — one sentinel file per repo proves the submodule is
/// actually checked out (empty submodule dirs aren't enough). typescript-eslint
/// is dropped: it's not in the M2 mining REPOS list (consolidate-gt.ts), so
/// filtering would yield zero tasks. TypeScript coverage stays via `nest`.
const FIXTURES: &[(&str, &str)] = &[
    ("gin", "context.go"),
    ("axum", "axum/Cargo.toml"),
    ("django", "django/__init__.py"),
    ("nest", "packages/core/package.json"),
];

#[test]
fn uc_impact_weighted_avg_across_fixtures() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    if !gt_path.exists() {
        eprintln!("[SKIP] GT missing: {}", gt_path.display());
        return;
    }
    let Ok(gt) = M2GroundTruth::load(&gt_path) else {
        eprintln!("[SKIP] M2 GT not loadable (sha256 or schema mismatch)");
        return;
    };
    assert_eq!(gt.uc, "impact");

    let mut weighted_sum = 0.0_f64;
    let mut total_tasks = 0_usize;
    let mut present = Vec::new();
    let mut skipped = Vec::new();

    for (name, sentinel) in FIXTURES {
        match run_fixture(name, sentinel, &gt) {
            Some((avg, n)) => {
                weighted_sum += avg * n as f64;
                total_tasks += n;
                present.push((*name, avg, n));
            }
            None => skipped.push(*name),
        }
    }

    let weighted_avg = if total_tasks == 0 {
        0.0
    } else {
        weighted_sum / total_tasks as f64
    };

    println!("\n╔═══ M2 GATE WEIGHTED AVG (dev split) ═══");
    for (name, avg, n) in &present {
        println!("║ {name:<20} n={n:>2} composite={avg:.3}");
    }
    if !skipped.is_empty() {
        println!("║ skipped: {}", skipped.join(", "));
    }
    println!("║ total tasks: {total_tasks}");
    println!("║ weighted composite: {weighted_avg:.3}");
    if weighted_avg >= 0.80 {
        println!("║ ✅ MEETS M2 target ≥0.80");
    } else {
        println!("║ ⚠️  GAP: {:.3}", 0.80 - weighted_avg);
    }
    println!("╚════════════════════════════\n");

    assert!(
        (0.0..=1.0).contains(&weighted_avg),
        "weighted composite out of range: {weighted_avg}"
    );
}
