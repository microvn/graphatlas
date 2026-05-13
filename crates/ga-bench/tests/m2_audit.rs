//! M2 audit — trace per-task diagnostics for 10 tasks to catch GT/engine
//! mismatches before concluding GA "fails" the gate.
//!
//! Prints for each task: seed, expected sets, GA actual sets, score
//! breakdown, and a pattern label (NoisySeed / PathMismatch / MissedTest /
//! EngineEmpty / Healthy). Lets a human eyeball whether the dataset or the
//! engine is at fault.
//!
//! Env: GA_M2_AUDIT_TASKS=10 (default) — how many tasks to trace.

use ga_bench::m2_ground_truth::{M2GroundTruth, Split};
use ga_bench::retriever::Retriever;
use ga_bench::retrievers::GaRetriever;
use ga_bench::score::impact_score;
use std::collections::HashMap;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

// S-002-bench: removed unused local `is_test_path` shim. Canonical at
// `ga_query::common::is_test_path` is the single source — m2_audit
// doesn't currently call it (audit operates on raw retriever output
// without source/test partition), so no import needed.

#[test]
fn m2_audit_traces_per_task() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");

    let Ok(gt) = M2GroundTruth::load(&gt_path) else {
        eprintln!("[SKIP] GT not loadable");
        return;
    };
    // Take dev split so we can safely "look at" the tasks
    let tasks: Vec<_> = gt.filter_split(Some(Split::Dev));
    let limit: usize = std::env::var("GA_M2_AUDIT_TASKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let mut by_repo: HashMap<String, Vec<_>> = HashMap::new();
    for t in &tasks {
        by_repo.entry(t.repo.clone()).or_default().push(*t);
    }

    let mut patterns: HashMap<&str, u32> = HashMap::new();
    let mut printed = 0usize;

    for (repo, tasks) in &by_repo {
        let fixture_dir = fixtures_root.join(repo);
        if !fixture_dir.exists() {
            continue;
        }
        if printed >= limit {
            break;
        }
        let cache = root.join(format!(".graphatlas-bench-cache/m2-audit/{repo}"));
        let mut ga = GaRetriever::new(cache);
        if let Err(e) = ga.setup(&fixture_dir) {
            eprintln!("[{repo}] setup failed: {e}");
            continue;
        }

        for task in tasks {
            if printed >= limit {
                break;
            }
            printed += 1;
            let q = serde_json::json!({
                "symbol": task.seed_symbol,
                "file": task.seed_file,
            });
            let (files, tests, max_d) = match ga.query_impact(&q) {
                Some(Ok(ia)) => (ia.files, ia.tests, Some(ia.max_depth)),
                _ => (Vec::new(), Vec::new(), None),
            };

            let score = impact_score(
                &files,
                &tests,
                max_d,
                &task.expected_files,
                &task.expected_tests,
                task.max_expected_depth,
                &task.should_touch_files,
            );

            // Diagnose pattern
            let files_set: std::collections::HashSet<&str> =
                files.iter().map(|s| s.as_str()).collect();
            let tests_set: std::collections::HashSet<&str> =
                tests.iter().map(|s| s.as_str()).collect();
            let expected_files_hit = task
                .expected_files
                .iter()
                .filter(|f| files_set.contains(f.as_str()))
                .count();
            let expected_tests_hit = task
                .expected_tests
                .iter()
                .filter(|t| tests_set.contains(t.as_str()))
                .count();

            let pattern: &str = if files.is_empty() && tests.is_empty() {
                "EngineEmpty"
            } else if expected_files_hit == 0 && !files.is_empty() {
                // Check if GA returned files that LOOK close (same basename or dir)
                let close_hit = task.expected_files.iter().any(|exp| {
                    let exp_base = exp.rsplit('/').next().unwrap_or(exp);
                    files.iter().any(|act| {
                        let act_base = act.rsplit('/').next().unwrap_or(act);
                        act_base == exp_base
                    })
                });
                if close_hit {
                    "PathMismatch"
                } else {
                    "EngineMissedFiles"
                }
            } else if expected_tests_hit == 0 && !task.expected_tests.is_empty() {
                "MissedTests"
            } else if score.composite >= 0.70 {
                "Healthy"
            } else if expected_files_hit > 0 && score.precision < 0.3 {
                "NoisyOutput"
            } else {
                "PartialHit"
            };
            *patterns.entry(pattern).or_insert(0) += 1;

            println!("\n─── [{}] {} ({})", task.repo, task.task_id, pattern);
            println!(
                "  subject:   {}",
                task.subject.chars().take(80).collect::<String>()
            );
            println!("  seed:      {} :: {}", task.seed_file, task.seed_symbol);
            println!(
                "  expected_files  ({}): {:?}",
                task.expected_files.len(),
                task.expected_files
            );
            println!(
                "  expected_tests  ({}): {:?}",
                task.expected_tests.len(),
                task.expected_tests
            );
            println!(
                "  GA actual_files ({}): {:?}",
                files.len(),
                files.iter().take(8).collect::<Vec<_>>()
            );
            if files.len() > 8 {
                println!("                       ... (+{} more)", files.len() - 8);
            }
            println!(
                "  GA actual_tests ({}): {:?}",
                tests.len(),
                tests.iter().take(6).collect::<Vec<_>>()
            );
            println!(
                "  hits: files {}/{}, tests {}/{}",
                expected_files_hit,
                task.expected_files.len(),
                expected_tests_hit,
                task.expected_tests.len(),
            );
            println!(
                "  score: composite={:.3} test_recall={:.3} completeness={:.3} precision={:.3} depth_f1={:.3}",
                score.composite, score.test_recall, score.completeness,
                score.precision, score.depth_f1,
            );
        }
        ga.teardown();
    }

    println!("\n╔═══ PATTERN DISTRIBUTION (N={}) ═══", printed);
    let mut sorted: Vec<_> = patterns.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (p, n) in sorted {
        println!("║ {:<22} {}", p, n);
    }
    println!("╚══════════════════════════════════════\n");
}
