//! H-M6 Co-change edge validation harness (2026-04-24).
//!
//! Tests whether wiring `signals::co_change` (dormant infra since EXP-014
//! revert) into impact output lifts blast_radius + adj_prec without blowing
//! noise budget.
//!
//! Algorithm (mirror `scripts/extract-seeds.ts:491-538`):
//!   1. For each seed, run `get_co_change_files(repo, seed_file, N, max_commit_size)`
//!   2. Filter: coChange count ≥ threshold (test 2 and 3)
//!   3. Exclude: files already in GA current output (would be redundant)
//!   4. Classify hits into 3 pools: expected_files / should_touch_files / neither
//!
//! Gate (per IMPACT_MACRO_STRATEGY.md §Phase 1 harness re-scope):
//!   - (new_expected + new_should_touch) : noise ≥ 1:5
//!   - Report per-pool so user sees which metric bitten.

use ga_bench::signals::co_change::{
    get_co_change_files, DEFAULT_MAX_COMMIT_SIZE, DEFAULT_N_COMMITS,
};
use ga_bench::signals::importers::{git_grep_importers, import_grep_spec};
use ga_bench::{M2GroundTruth, Split};
use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::collections::HashSet;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn current_impacted(store: &Store, symbol: &str) -> HashSet<String> {
    impact(
        store,
        &ImpactRequest {
            symbol: Some(symbol.into()),
            include_break_points: Some(false),
            include_routes: Some(false),
            include_configs: Some(false),
            include_risk: Some(false),
            ..Default::default()
        },
    )
    .map(|r| r.impacted_files.into_iter().map(|f| f.path).collect())
    .unwrap_or_default()
}

/// docs / changelog / build excludes borrowed from EXP-015 lessons.
fn is_excluded_noise(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".rst")
        || lower.ends_with(".txt")
        || lower.starts_with("docs/")
        || lower.contains("/docs/")
        || lower.starts_with("changelog")
        || lower.contains("/changelog")
}

#[test]
#[ignore]
fn m2_cochange_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-cochange-validate");

    if !gt_path.exists() {
        eprintln!("[SKIP] no GT");
        return;
    }
    let Ok(gt) = M2GroundTruth::load(&gt_path) else {
        eprintln!("[SKIP] GT not loadable");
        return;
    };

    let dev: Vec<&ga_bench::M2Task> = gt.tasks.iter().filter(|t| t.split == Split::Dev).collect();
    let mut by_repo: std::collections::HashMap<String, Vec<&ga_bench::M2Task>> =
        std::collections::HashMap::new();
    for t in &dev {
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }

    // Test 5 variants:
    //  A: co-change ≥2 only (baseline, already ran in earlier iteration)
    //  A': co-change ≥3 only (stricter baseline)
    //  B: importers ∩ co-change ≥2 (OPTION A — mimic GT Phase A∩B)
    //  B': importers ∩ co-change ≥3 (stricter intersection)
    //  H: Hybrid H1 = A' ∪ B  (union cc≥3 broad signal with importers∩cc≥2 tight)
    //
    // Mode encoding: (label, threshold, mode)
    //   mode=0: cc-only
    //   mode=1: importers ∩ cc
    //   mode=2: hybrid (A' ∪ B) — threshold param unused
    let variants: &[(&str, u32, u8)] = &[
        ("cc-only ≥2", 2, 0),
        ("cc-only ≥3", 3, 0),
        ("importers ∩ cc ≥2", 2, 1),
        ("importers ∩ cc ≥3", 3, 1),
        ("HYBRID A'∪B", 0, 2),
    ];

    for (label, threshold, mode) in variants {
        println!("\n=== H-M6 CO-CHANGE VALIDATION ({}, 3-pool) ===", label);

        let mut total_exp = 0usize;
        let mut total_stf = 0usize;
        let mut already_exp = 0usize;
        let mut new_exp = 0usize;
        let mut already_stf = 0usize;
        let mut new_stf = 0usize;
        let mut noise = 0usize;
        let mut per_repo: Vec<(String, usize, usize, usize, usize, usize, usize, usize)> =
            Vec::new();

        for (repo, tasks) in &by_repo {
            let fixture = fixtures_root.join(repo);
            if !fixture.exists() {
                continue;
            }
            let cache = cache_root.join(repo);
            let _ = std::fs::remove_dir_all(&cache);
            let Ok(store) = Store::open_with_root(&cache, &fixture) else {
                continue;
            };
            if build_index(&store, &fixture).is_err() {
                continue;
            }

            let mut r_exp = 0;
            let mut r_stf = 0;
            let mut r_ae = 0;
            let mut r_ne = 0;
            let mut r_as = 0;
            let mut r_ns = 0;
            let mut r_noise = 0;

            for task in tasks {
                let expected: HashSet<&String> = task.expected_files.iter().collect();
                let should_touch: HashSet<&String> = task.should_touch_files.iter().collect();
                let current = current_impacted(&store, &task.seed_symbol);

                // Co-change from seed_file at HEAD (working-tree commit proxy).
                // Note: extract-seeds.ts uses baseCommit explicitly; bench here
                // uses fixture HEAD because we don't check out baseCommit per
                // task (would be expensive). Acceptable for upper-bound signal.
                let cc_map = get_co_change_files(
                    &fixture,
                    &task.seed_file,
                    DEFAULT_N_COMMITS,
                    DEFAULT_MAX_COMMIT_SIZE,
                );
                // Build cc sets at both thresholds once per task — reused in hybrid.
                let cc2_set: HashSet<String> = cc_map
                    .iter()
                    .filter(|(_, n)| **n >= 2)
                    .map(|(f, _)| f.clone())
                    .filter(|f| !is_excluded_noise(f))
                    .collect();
                let cc3_set: HashSet<String> = cc_map
                    .iter()
                    .filter(|(_, n)| **n >= 3)
                    .map(|(f, _)| f.clone())
                    .filter(|f| !is_excluded_noise(f))
                    .collect();

                let cc_signal: HashSet<String> = match mode {
                    0 => {
                        // cc-only: pick set by threshold
                        if *threshold >= 3 {
                            cc3_set.clone()
                        } else {
                            cc2_set.clone()
                        }
                    }
                    1 => {
                        // importers ∩ cc(threshold)
                        let importers =
                            match import_grep_spec(&fixture, &task.seed_file, &task.lang) {
                                Some(spec) => git_grep_importers(&fixture, &spec),
                                None => HashSet::new(),
                            };
                        let base = if *threshold >= 3 { &cc3_set } else { &cc2_set };
                        base.intersection(&importers).cloned().collect()
                    }
                    _ => {
                        // Hybrid H1: A' (cc≥3) ∪ B (importers ∩ cc≥2)
                        let importers =
                            match import_grep_spec(&fixture, &task.seed_file, &task.lang) {
                                Some(spec) => git_grep_importers(&fixture, &spec),
                                None => HashSet::new(),
                            };
                        let b: HashSet<String> =
                            cc2_set.intersection(&importers).cloned().collect();
                        cc3_set.union(&b).cloned().collect()
                    }
                };

                for ef in &task.expected_files {
                    r_exp += 1;
                    if current.contains(ef.as_str()) {
                        r_ae += 1;
                    } else if cc_signal.contains(ef.as_str()) {
                        r_ne += 1;
                    }
                }
                for sf in &task.should_touch_files {
                    r_stf += 1;
                    if current.contains(sf.as_str()) {
                        r_as += 1;
                    } else if cc_signal.contains(sf.as_str()) {
                        r_ns += 1;
                    }
                }
                for s in &cc_signal {
                    if expected.contains(s) || should_touch.contains(s) {
                        continue;
                    }
                    if current.contains(s) {
                        continue;
                    }
                    r_noise += 1;
                }
            }
            per_repo.push((repo.clone(), r_exp, r_stf, r_ae, r_ne, r_as, r_ns, r_noise));
            total_exp += r_exp;
            total_stf += r_stf;
            already_exp += r_ae;
            new_exp += r_ne;
            already_stf += r_as;
            new_stf += r_ns;
            noise += r_noise;
        }

        println!(
            "{:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}",
            "repo", "|exp|", "|stf|", "a.exp", "n.exp", "a.stf", "n.stf", "noise"
        );
        for (r, e, s, ae, ne, as_, ns, nz) in &per_repo {
            println!(
                "{:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}",
                r, e, s, ae, ne, as_, ns, nz
            );
        }
        println!(
            "{:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}",
            "TOTAL", total_exp, total_stf, already_exp, new_exp, already_stf, new_stf, noise
        );

        let new_total = new_exp + new_stf;
        let sig_noise = if noise == 0 {
            f64::INFINITY
        } else {
            new_total as f64 / noise as f64
        };
        let pct_ne = if total_exp > 0 {
            new_exp as f64 / total_exp as f64 * 100.0
        } else {
            0.0
        };
        let pct_ns = if total_stf > 0 {
            new_stf as f64 / total_stf as f64 * 100.0
        } else {
            0.0
        };

        println!(
            "\nCOMPOSITE lift:    +{:.1}% expected ({} new)",
            pct_ne, new_exp
        );
        println!(
            "BLAST_RADIUS lift: +{:.1}% should_touch ({} new)",
            pct_ns, new_stf
        );
        println!(
            "SIGNAL:NOISE       {} new / {} noise = {:.2}",
            new_total, noise, sig_noise
        );
        let go = sig_noise >= 0.2 && new_total >= 3;
        println!(
            "GATE (s/n ≥ 0.2, new ≥ 3): {}",
            if go { "✅ IMPLEMENT" } else { "❌ SKIP" }
        );
    }
}
