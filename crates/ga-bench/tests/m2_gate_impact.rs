//! M2 gate integration test — runs impact benchmark per S-004 AS-009.
//!
//! Loads `benches/uc-impact/ground-truth.json` (SHA256 verified), runs 4
//! in-process retrievers (ga, bm25, random, ripgrep) on all tasks, emits
//! `bench-results/impact-*-leaderboard.md` + `impact-aggregate.md`.
//!
//! Environment knobs:
//!   GA_M2_SPLIT    — "dev" | "test" | "all" (default: "test")
//!   GA_M2_REPOS    — comma-separated repo subset (default: all 5)
//!   GA_M2_PIN      — "0" to disable commit pinning (default: on per
//!                    docs/guide/uc-impact-dataset-methodology.md §Commit-pinning;
//!                    flipped 2026-04-26 to fix mockito M2 Test Recall)
//!   GA_M2_RETRIEVERS — comma-separated retriever subset
//!                     (default: "ga,bm25,random,ripgrep")
//!
//! **Does not gate-assert** — prints the table and lets CI/human judge.
//! Once engine is tuned, a separate gate test can assert composite ≥ 0.80.

use ga_bench::m2_markdown::write_reports;
use ga_bench::m2_runner::{run, RunOpts};
use ga_bench::{M2GroundTruth, Split};
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn m2_gate_impact() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2");
    let output_dir = root.join("bench-results");

    if !gt_path.exists() {
        eprintln!("[SKIP] ground-truth.json not found — run scripts/consolidate-gt.ts");
        return;
    }

    // Verify SHA256 up front (also done in M2GroundTruth::load but we want
    // an early skip path if the dataset isn't present yet).
    let gt = match M2GroundTruth::load(&gt_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[SKIP] GT load failed: {e}");
            return;
        }
    };
    println!(
        "Loaded {} tasks from GT (schema v{})",
        gt.tasks.len(),
        gt.schema_version
    );

    // Check which fixtures are actually checked out
    let present_repos: Vec<&str> = gt
        .per_repo
        .keys()
        .filter(|r| {
            let dir = fixtures_root.join(r);
            dir.exists()
                && std::fs::read_dir(&dir)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
        })
        .map(|s| s.as_str())
        .collect();

    if present_repos.is_empty() {
        eprintln!(
            "[SKIP] no fixtures checked out under {}",
            fixtures_root.display()
        );
        return;
    }
    println!("Present fixtures: {:?}", present_repos);

    let split = match std::env::var("GA_M2_SPLIT").as_deref() {
        Ok("all") => None,
        Ok("dev") => Some(Split::Dev),
        Ok("test") | Err(_) => Some(Split::Test),
        Ok(other) => {
            panic!("GA_M2_SPLIT must be dev|test|all (got {other})");
        }
    };

    let retrievers: Vec<String> = std::env::var("GA_M2_RETRIEVERS")
        .unwrap_or_else(|_| {
            "ga,codegraphcontext,codebase-memory,code-review-graph,bm25,ripgrep,random".to_string()
        })
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let pin_commits = resolve_pin_commits();

    let opts = RunOpts {
        gt_path: gt_path.clone(),
        fixtures_root: fixtures_root.clone(),
        cache_root,
        split,
        retrievers,
        pin_commits,
        output_dir: output_dir.clone(),
    };

    let report = match run(opts) {
        Ok(r) => r,
        Err(e) => {
            panic!("bench run failed: {e}");
        }
    };

    println!("\n=== M2 AGGREGATE ===");
    for entry in &report.aggregate {
        println!(
            "  {:<10} composite={:.3}  test_recall={:.3}  completeness={:.3}  precision={:.3}  blast_radius={:.3}  adj_prec={:.3}  p95={}ms  pass={:.1}%",
            entry.retriever,
            entry.mean_composite,
            entry.mean_test_recall,
            entry.mean_completeness,
            entry.mean_precision,
            entry.mean_blast_radius_coverage,
            entry.mean_adjusted_precision,
            entry.p95_latency_ms,
            entry.pass_rate * 100.0,
        );
    }

    match write_reports(&report, &output_dir) {
        Ok(()) => println!("\nWrote leaderboards to {}", output_dir.display()),
        Err(e) => eprintln!("WARN: failed to write reports: {e}"),
    }

    // Print gate status (non-asserting — engine tuning comes later)
    let ga = report.aggregate.iter().find(|e| e.retriever == "ga");
    if let Some(ga) = ga {
        println!("\n╔═══ M2 GATE (S-004 AS-009) ═══");
        println!("║ GA composite:  {:.3}   target ≥ 0.80", ga.mean_composite);
        println!(
            "║ test_recall:   {:.3}   target ≥ 0.85",
            ga.mean_test_recall
        );
        println!(
            "║ completeness:  {:.3}   target ≥ 0.80",
            ga.mean_completeness
        );
        println!("║ depth_F1:      {:.3}   target ≥ 0.80", ga.mean_depth_f1);
        println!("║ precision:     {:.3}   target ≥ 0.70", ga.mean_precision);
        println!("║ p95 latency:   {}ms   target ≤ 500ms", ga.p95_latency_ms);
        println!(
            "║ blast_radius:  {:.3}   (supplementary)",
            ga.mean_blast_radius_coverage
        );
        println!(
            "║ adj_precision: {:.3}   (supplementary)",
            ga.mean_adjusted_precision
        );
        if ga.mean_composite >= 0.80 {
            println!("║ ✅ GATE MEETS target");
        } else {
            println!("║ ⚠️  GAP: {:.3}", 0.80 - ga.mean_composite);
        }
        println!("╚═══════════════════════════════\n");
    }
}

// ─── pin_commits flag-resolution tests ──────────────────────────────
//
// Lock the contract: M2 gate MUST default to pin_commits=on so that the
// bench indexes the submodule at each task's `base_commit`, matching
// the GT-mining methodology (docs/guide/uc-impact-dataset-methodology.md
// §"Commit pinning"). Pre-2026-04-26 this was opt-in via GA_M2_PIN=1
// and that misalignment cost mockito M2 ~0.07 Test Recall.
//
// These tests check the resolution expression in isolation so they
// stay fast (<1s) and don't depend on fixture submodules. The actual
// behavior is exercised by the slow `m2_gate_impact` test above.

fn resolve_pin_commits() -> bool {
    pin_from_env(std::env::var("GA_M2_PIN").ok().as_deref())
}

/// Pure resolution of the `GA_M2_PIN` flag — split from
/// `resolve_pin_commits` so unit tests can exercise the policy without
/// mutating process env (`std::env::set_var` is not thread-safe and
/// races when multiple tests run in parallel).
///
/// 2026-04-26 default flipped ON per docs/guide/uc-impact-dataset-methodology.md
/// §"Commit pinning" — the bench now indexes each fixture at the
/// task's `base_commit` so co-change history + Symbol graph match
/// the GT-mining-time state. `GA_M2_PIN=0` is the opt-out for fast
/// developer iteration that doesn't need pin accuracy.
fn pin_from_env(value: Option<&str>) -> bool {
    value != Some("0")
}

#[test]
fn pin_commits_default_is_on() {
    assert!(
        pin_from_env(None),
        "M2 gate must default to pin_commits=on per methodology §Commit pinning"
    );
}

#[test]
fn pin_commits_opt_out_via_env_zero() {
    assert!(
        !pin_from_env(Some("0")),
        "GA_M2_PIN=0 must disable pinning (escape hatch)"
    );
}

#[test]
fn pin_commits_explicit_one_still_enables() {
    // Backward-compat: callers that previously set GA_M2_PIN=1 to opt-in
    // must keep working after the default flip.
    assert!(
        pin_from_env(Some("1")),
        "GA_M2_PIN=1 must still enable pinning (backward-compat)"
    );
}

#[test]
fn pin_commits_other_values_default_to_on() {
    // Defensive: any value other than "0" enables pinning. Prevents
    // typo-as-disable (e.g., GA_M2_PIN=false would NOT disable).
    assert!(pin_from_env(Some("false")));
    assert!(pin_from_env(Some("")));
    assert!(pin_from_env(Some("yes")));
}
