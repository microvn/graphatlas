//! M1 gate verification for the 3 previously-unmeasured UCs: `callees`,
//! `symbols`, `file_summary`.
//!
//! Generates GT on-the-fly via the new H2/H3/H4 raw-AST rules, runs the
//! `ga` retriever against each (fixture, UC) combo under
//! `benches/fixtures/`, and prints an honest per-fixture + per-UC score
//! table.
//!
//! Paired with pre-existing callers + importers leaderboards in
//! `bench-results/`, this closes the M1 gate evaluation ("GA+ dominates
//! ≥3/5 Layer-1 UCs").

use ga_bench::gt_gen::{
    generate_gt, h2_callees_text::H2CalleesText, h3_symbols_exact::H3SymbolsExact,
    h4_file_summary_basic::H4FileSummaryBasic, GtRule,
};
use ga_bench::retriever::Retriever;
use ga_bench::retrievers::GaRetriever;
use ga_bench::score::{f1, mrr};
use ga_index::Store;
use ga_query::indexer::build_index;
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

fn fixtures_with_sources() -> Vec<&'static str> {
    let root = workspace_root();
    // v1.1-M4 — mockito added when S-001 Java shipped (commit c61b611).
    // S-002-bench — kotlinx-coroutines + kotlinx-serialization added per
    // Lang-C1 Kotlin contract closure (AS-007 saturation in coroutines /
    // Lang-C7 saturation in serialization).
    // S-004-bench — jekyll + faraday added per Lang-C1 Ruby closure
    // (jekyll: Minitest test_*.rb prefix + 0.6-I metaprogramming via
    // Liquid plugin DSL; faraday: RSpec _spec.rb suffix + middleware
    // chain DSL — orthogonal-domain N=2 per L15 single-fixture-bias rule).
    // Per Lang-C1, a new lang must pass M1 + M2 gates against a real OSS
    // fixture before the lang counts as "shipped".
    [
        "axum",
        "gin",
        "httpx",
        "preact",
        "radash",
        "mockito",
        "kotlinx-coroutines",
        "kotlinx-serialization",
        "MQTTnet",
        "Polly",
        "jekyll",
        "faraday",
    ]
    .into_iter()
    .filter(|name| {
        let dir = root.join("benches/fixtures").join(name);
        dir.exists()
            && std::fs::read_dir(&dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false)
    })
    .collect()
}

struct UcResult {
    uc: &'static str,
    fixture: &'static str,
    tasks: usize,
    primary: f64, // F1 for set-based, MRR for symbols
    passed: usize,
}

#[test]
fn m1_gate_measures_callees_symbols_file_summary() {
    let fixtures = fixtures_with_sources();
    if fixtures.is_empty() {
        eprintln!("[SKIP] no fixture submodules checked out under benches/fixtures/");
        return;
    }

    println!("\n=== M1 gate measurement: callees + symbols + file_summary ===");
    println!("Fixtures: {fixtures:?}");

    let mut rows: Vec<UcResult> = Vec::new();

    for fixture in &fixtures {
        let root = workspace_root();
        let fixture_dir = root.join("benches/fixtures").join(fixture);
        let tmp = tempfile::TempDir::new().unwrap();

        // Open store + index once per fixture for GT generation. DON'T
        // pre-create the dir — Store::open_with_root requires 0700 and
        // fs::create_dir_all respects umask (0755 on macOS, rejected).
        let gt_cache = tmp.path().join("gt_cache");
        let _ = std::fs::remove_dir_all(&gt_cache);
        let store = Store::open_with_root(&gt_cache, &fixture_dir).unwrap();
        if build_index(&store, &fixture_dir).is_err() {
            eprintln!("[SKIP fixture {fixture}] build_index failed");
            continue;
        }

        // Setup retriever with its own separate cache subdir.
        let ret_cache = tmp.path().join("ret_cache");
        let mut ret = GaRetriever::new(ret_cache);
        ret.setup(&fixture_dir).unwrap();

        for (uc, rule, is_mrr) in [
            (
                "callees",
                Box::new(H2CalleesText {
                    exclude_tests: true,
                }) as Box<dyn GtRule>,
                false,
            ),
            ("symbols", Box::new(H3SymbolsExact) as Box<dyn GtRule>, true),
            (
                "file_summary",
                Box::new(H4FileSummaryBasic {
                    exclude_tests: true,
                }) as Box<dyn GtRule>,
                false,
            ),
        ] {
            let rules = vec![rule];
            let gt = match generate_gt(uc, fixture, &store, &fixture_dir, &rules) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("[SKIP {fixture}/{uc}] generate_gt: {e}");
                    continue;
                }
            };
            if gt.tasks.is_empty() {
                eprintln!("[SKIP {fixture}/{uc}] no tasks generated");
                continue;
            }

            let mut scores: Vec<f64> = Vec::new();
            let mut passed = 0usize;
            for task in &gt.tasks {
                let actual = match ret.query(uc, &task.query) {
                    Ok(a) => a,
                    Err(_) => {
                        scores.push(0.0);
                        continue;
                    }
                };
                let score = if is_mrr {
                    let target = task.expected.first().cloned().unwrap_or_default();
                    let act_refs: Vec<&str> = actual.iter().map(String::as_str).collect();
                    mrr(&act_refs, &target.as_str())
                } else {
                    let exp: Vec<&str> = task.expected.iter().map(String::as_str).collect();
                    let act: Vec<&str> = actual.iter().map(String::as_str).collect();
                    f1(&exp, &act)
                };
                if score >= 0.5 {
                    passed += 1;
                }
                scores.push(score);
            }
            let primary = scores.iter().sum::<f64>() / scores.len() as f64;
            rows.push(UcResult {
                uc,
                fixture,
                tasks: gt.tasks.len(),
                primary,
                passed,
            });
        }
    }

    // Per-UC aggregation.
    println!("\n--- Per (UC, fixture) ---");
    println!(
        "{:<14} {:<10} {:>6} {:>8} {:>7}",
        "uc", "fixture", "tasks", "score", "pass%"
    );
    for r in &rows {
        let pass_pct = 100.0 * r.passed as f64 / r.tasks.max(1) as f64;
        println!(
            "{:<14} {:<10} {:>6} {:>8.3} {:>6.1}%",
            r.uc, r.fixture, r.tasks, r.primary, pass_pct
        );
    }

    for uc in ["callees", "symbols", "file_summary"] {
        let slice: Vec<&UcResult> = rows.iter().filter(|r| r.uc == uc).collect();
        if slice.is_empty() {
            continue;
        }
        let total_tasks: usize = slice.iter().map(|r| r.tasks).sum();
        let weighted = slice
            .iter()
            .map(|r| r.primary * r.tasks as f64)
            .sum::<f64>()
            / total_tasks.max(1) as f64;
        let total_pass: usize = slice.iter().map(|r| r.passed).sum();
        let pass_pct = 100.0 * total_pass as f64 / total_tasks.max(1) as f64;
        println!(
            "  UC {:<14} weighted={:.3} over {} tasks — pass@0.5 = {:.1}%",
            uc, weighted, total_tasks, pass_pct
        );
    }

    assert!(
        !rows.is_empty(),
        "at least one fixture must have yielded a row"
    );
}
