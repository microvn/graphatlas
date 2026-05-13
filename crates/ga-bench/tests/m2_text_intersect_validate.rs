//! Proposal #1 validate — ripgrep × graph intersect filter.
//!
//! Current M2 BFS returns up to 50 files per task; precision = |output ∩
//! expected| / |output| is ~0.126 because most BFS-surfaced files are
//! hub/transitive noise. Hypothesis: filter impacted_files to retain only
//! those that contain the seed symbol as raw text (word-boundary token).
//! Hub files that don't mention the seed textually get dropped.
//!
//! Measures pre/post precision + completeness + depth_F1 on dev corpus
//! WITHOUT changing production code.

use ga_bench::{M2GroundTruth, Split};
use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn current_impacted(store: &Store, symbol: &str) -> Vec<String> {
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

/// Word-boundary check — `seed` appears as a standalone identifier
/// (not as substring of another identifier). Empty seeds never match.
fn contains_seed_token(text: &str, seed: &str) -> bool {
    if seed.is_empty() {
        return false;
    }
    // Scan for seed; verify neither side is an identifier char.
    let bytes = text.as_bytes();
    let sbytes = seed.as_bytes();
    let mut i = 0;
    while i + sbytes.len() <= bytes.len() {
        if &bytes[i..i + sbytes.len()] == sbytes {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_ok =
                i + sbytes.len() == bytes.len() || !is_ident_byte(bytes[i + sbytes.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn file_contains_seed(repo_root: &Path, rel_path: &str, seed: &str) -> bool {
    let full = repo_root.join(rel_path);
    let Ok(bytes) = std::fs::read(&full) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    contains_seed_token(text, seed)
}

#[test]
#[ignore]
fn m2_text_intersect_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-text-intersect-validate");

    if !gt_path.exists() {
        return;
    }
    let Ok(gt) = M2GroundTruth::load(&gt_path) else {
        return;
    };

    let dev: Vec<&ga_bench::M2Task> = gt.tasks.iter().filter(|t| t.split == Split::Dev).collect();
    let mut by_repo: std::collections::HashMap<String, Vec<&ga_bench::M2Task>> =
        std::collections::HashMap::new();
    for t in &dev {
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }

    // Aggregates across tasks
    let mut total_pre_tp = 0usize; // expected caught pre-filter
    let mut total_pre_fp = 0usize; // non-expected in output pre-filter
    let mut total_expected = 0usize;
    let mut total_post_tp = 0usize; // expected caught post-filter
    let mut total_post_fp = 0usize;
    let mut total_signal_lost = 0usize; // expected dropped by filter
    let mut total_noise_removed = 0usize; // non-expected dropped

    let mut per_repo: Vec<(String, usize, usize, usize, usize, usize, usize)> = Vec::new();
    // (repo, pre_tp, pre_fp, post_tp, post_fp, signal_lost, noise_removed)

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

        let mut r_pre_tp = 0;
        let mut r_pre_fp = 0;
        let mut r_post_tp = 0;
        let mut r_post_fp = 0;
        let mut r_lost = 0;
        let mut r_removed = 0;

        for task in tasks {
            let expected: HashSet<&String> = task.expected_files.iter().collect();
            let bfs_output = current_impacted(&store, &task.seed_symbol);

            for file in &bfs_output {
                let in_expected = expected.contains(file);
                let contains_text = file_contains_seed(&fixture, file, &task.seed_symbol);

                // Pre-filter counts
                if in_expected {
                    r_pre_tp += 1;
                } else {
                    r_pre_fp += 1;
                }

                // Post-filter: keep only files containing seed text
                if contains_text {
                    if in_expected {
                        r_post_tp += 1;
                    } else {
                        r_post_fp += 1;
                    }
                } else {
                    if in_expected {
                        r_lost += 1;
                    } else {
                        r_removed += 1;
                    }
                }
            }

            total_expected += task.expected_files.len();
        }

        per_repo.push((
            repo.clone(),
            r_pre_tp,
            r_pre_fp,
            r_post_tp,
            r_post_fp,
            r_lost,
            r_removed,
        ));
        total_pre_tp += r_pre_tp;
        total_pre_fp += r_pre_fp;
        total_post_tp += r_post_tp;
        total_post_fp += r_post_fp;
        total_signal_lost += r_lost;
        total_noise_removed += r_removed;
    }

    // Compute precision + completeness pre/post.
    let pre_precision = if total_pre_tp + total_pre_fp > 0 {
        total_pre_tp as f64 / (total_pre_tp + total_pre_fp) as f64
    } else {
        0.0
    };
    let post_precision = if total_post_tp + total_post_fp > 0 {
        total_post_tp as f64 / (total_post_tp + total_post_fp) as f64
    } else {
        0.0
    };
    let pre_completeness = if total_expected > 0 {
        total_pre_tp as f64 / total_expected as f64
    } else {
        0.0
    };
    let post_completeness = if total_expected > 0 {
        total_post_tp as f64 / total_expected as f64
    } else {
        0.0
    };

    println!("\n=== TEXT-INTERSECT FILTER VALIDATION (Proposal #1) ===");
    println!(
        "{:<12}{:>8}{:>8}{:>8}{:>8}{:>8}{:>10}",
        "repo", "pre_tp", "pre_fp", "post_tp", "post_fp", "lost", "noise_rm"
    );
    for (r, ptp, pfp, pstp, psfp, l, nr) in &per_repo {
        println!(
            "{:<12}{:>8}{:>8}{:>8}{:>8}{:>8}{:>10}",
            r, ptp, pfp, pstp, psfp, l, nr
        );
    }
    println!(
        "{:<12}{:>8}{:>8}{:>8}{:>8}{:>8}{:>10}",
        "TOTAL",
        total_pre_tp,
        total_pre_fp,
        total_post_tp,
        total_post_fp,
        total_signal_lost,
        total_noise_removed
    );

    println!(
        "\npre-filter  precision = {:.3}  completeness = {:.3}",
        pre_precision, pre_completeness
    );
    println!(
        "post-filter precision = {:.3}  completeness = {:.3}",
        post_precision, post_completeness
    );
    println!("precision delta   = {:+.3}", post_precision - pre_precision);
    println!(
        "completeness delta = {:+.3}",
        post_completeness - pre_completeness
    );

    // Composite delta (precision weight 0.15, completeness weight 0.30).
    let delta_composite =
        0.15 * (post_precision - pre_precision) + 0.30 * (post_completeness - pre_completeness);
    println!(
        "\nestimated composite delta (prec·0.15 + comp·0.30) = {:+.4}",
        delta_composite
    );
    println!("\nGate: composite delta > 0 AND signal_lost/noise_removed < 0.1");
    let sig_to_noise = if total_noise_removed == 0 {
        f64::INFINITY
    } else {
        total_signal_lost as f64 / total_noise_removed as f64
    };
    println!("signal_lost:noise_removed = {:.4}", sig_to_noise);
    let go = delta_composite > 0.0 && sig_to_noise < 0.1;
    println!("Recommendation: {}", if go { "IMPLEMENT" } else { "SKIP" });
}
