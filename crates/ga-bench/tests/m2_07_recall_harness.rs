//! M2-07-VALIDATE — recall validation harness for fuzzy symbol-name match.
//!
//! Measures: on dev corpus (30 tasks, 5 repos), if we added a fuzzy
//! symbol-name match layer on top of current M2-05 affected_tests pipeline,
//! how many expected_tests that `path_mentions` currently misses WOULD be
//! surfaced by checking contained function/method symbol names?
//!
//! Upper-bound estimate — assumes perfect trigram/FTS recall (any name that
//! contains seed or any stem is found). If this upper bound is small, M2-07
//! is not worth implementing.
//!
//! Also counts potential false positives: test files NOT in GT expected_tests
//! that WOULD surface under the same fuzzy rule.
//!
//! Trigger: `cargo test -p ga-bench --release --test m2_07_recall_harness -- \
//!          --ignored --nocapture`

use ga_bench::{M2GroundTruth, Split};
use ga_index::Store;
use ga_query::indexer::build_index;
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

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`. Was a stale local copy lacking
// Java/Kotlin/C#/Ruby suffixes + KMP multi-target dirs.
use ga_query::common::is_test_path;

fn file_stem(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = name.split('.').next()?;
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

fn path_mentions(path: &str, symbol: &str, stems: &[String]) -> bool {
    if path.contains(symbol) {
        return true;
    }
    stems.iter().any(|s| !s.is_empty() && path.contains(s))
}

/// Load all test-file symbol names via DEFINES edges.
fn collect_test_file_symbol_names(
    conn: &lbug::Connection<'_>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let Ok(rs) = conn.query(
        "MATCH (f:File)-[:DEFINES]->(s:Symbol) \
         WHERE s.kind IN ['function', 'method'] \
         RETURN f.path, s.name",
    ) else {
        return map;
    };
    for row in rs {
        let vals: Vec<_> = row.into_iter().collect();
        let (Some(lbug::Value::String(path)), Some(lbug::Value::String(name))) =
            (vals.first().cloned(), vals.get(1).cloned())
        else {
            continue;
        };
        if !is_test_path(&path) {
            continue;
        }
        map.entry(path).or_default().push(name);
    }
    map
}

fn fn_name_mentions(names: Option<&Vec<String>>, seed: &str, stems: &[String]) -> bool {
    let Some(names) = names else { return false };
    names
        .iter()
        .any(|n| n.contains(seed) || stems.iter().any(|s| !s.is_empty() && n.contains(s)))
}

#[test]
#[ignore]
fn m2_07_recall_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-07-harness");

    if !gt_path.exists() {
        eprintln!("[SKIP] ground-truth.json not found");
        return;
    }
    let gt = match M2GroundTruth::load(&gt_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[SKIP] GT load failed: {e}");
            return;
        }
    };
    println!(
        "Loaded {} tasks (schema v{})",
        gt.tasks.len(),
        gt.schema_version
    );

    // Dev split only
    let dev: Vec<_> = gt.tasks.iter().filter(|t| t.split == Split::Dev).collect();
    println!("Dev split: {} tasks", dev.len());

    // Group by repo to build each fixture exactly once
    let mut by_repo: std::collections::HashMap<String, Vec<&ga_bench::M2Task>> =
        std::collections::HashMap::new();
    for t in &dev {
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }

    // Aggregate counters
    let mut total_expected_tests = 0usize;
    let mut caught_by_path_mentions = 0usize;
    let mut new_caught_by_fuzzy = 0usize;
    let mut missed_by_both = 0usize;
    let mut noise_fuzzy_additions = 0usize;

    let mut per_repo: Vec<(String, usize, usize, usize, usize, usize)> = Vec::new();
    // (repo, total_gt_tests, already_caught, new_caught, missed, noise)

    for (repo, tasks) in &by_repo {
        let fixture = fixtures_root.join(repo);
        if !fixture.exists() {
            eprintln!("[SKIP repo={}] fixture missing", repo);
            continue;
        }
        let cache = cache_root.join(repo);
        let _ = std::fs::remove_dir_all(&cache);
        let store = match Store::open_with_root(&cache, &fixture) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[SKIP repo={}] open store: {e}", repo);
                continue;
            }
        };
        if let Err(e) = build_index(&store, &fixture) {
            eprintln!("[SKIP repo={}] build_index: {e}", repo);
            continue;
        }
        let conn = store.connection().expect("conn");
        let test_fn_names = collect_test_file_symbol_names(&conn);
        println!(
            "[{}] indexed; {} test files with symbols",
            repo,
            test_fn_names.len()
        );

        let mut r_total = 0usize;
        let mut r_caught = 0usize;
        let mut r_new = 0usize;
        let mut r_missed = 0usize;
        let mut r_noise = 0usize;

        for task in tasks {
            // Compute seed_stems = file stems of seed_file (depth=0 impacted).
            let stems: Vec<String> = if let Some(s) = file_stem(&task.seed_file) {
                vec![s]
            } else {
                vec![]
            };

            // Classify each expected test
            let expected_set: HashSet<&String> = task.expected_tests.iter().collect();
            for expected in &task.expected_tests {
                r_total += 1;
                if path_mentions(expected, &task.seed_symbol, &stems) {
                    r_caught += 1;
                } else if fn_name_mentions(test_fn_names.get(expected), &task.seed_symbol, &stems)
                    && task.seed_symbol.len() >= 5
                {
                    r_new += 1;
                } else {
                    r_missed += 1;
                }
            }

            // Noise: test files NOT in expected_tests but matched by fuzzy
            if task.seed_symbol.len() >= 5 {
                for (test_path, names) in &test_fn_names {
                    if expected_set.contains(test_path) {
                        continue;
                    }
                    if path_mentions(test_path, &task.seed_symbol, &stems) {
                        continue;
                    }
                    if names.iter().any(|n| {
                        n.contains(&task.seed_symbol)
                            || stems.iter().any(|s| !s.is_empty() && n.contains(s))
                    }) {
                        r_noise += 1;
                    }
                }
            }
        }

        per_repo.push((repo.clone(), r_total, r_caught, r_new, r_missed, r_noise));
        total_expected_tests += r_total;
        caught_by_path_mentions += r_caught;
        new_caught_by_fuzzy += r_new;
        missed_by_both += r_missed;
        noise_fuzzy_additions += r_noise;
    }

    println!("\n=== M2-07 RECALL VALIDATION ===");
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "repo", "total", "path", "new", "missed", "noise"
    );
    for (r, t, c, n, m, ns) in &per_repo {
        println!("{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}", r, t, c, n, m, ns);
    }
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "TOTAL",
        total_expected_tests,
        caught_by_path_mentions,
        new_caught_by_fuzzy,
        missed_by_both,
        noise_fuzzy_additions
    );

    let frac_already = if total_expected_tests > 0 {
        caught_by_path_mentions as f64 / total_expected_tests as f64 * 100.0
    } else {
        0.0
    };
    let frac_new = if total_expected_tests > 0 {
        new_caught_by_fuzzy as f64 / total_expected_tests as f64 * 100.0
    } else {
        0.0
    };
    let frac_missed = if total_expected_tests > 0 {
        missed_by_both as f64 / total_expected_tests as f64 * 100.0
    } else {
        0.0
    };
    let signal_to_noise = if noise_fuzzy_additions == 0 {
        f64::INFINITY
    } else {
        new_caught_by_fuzzy as f64 / noise_fuzzy_additions as f64
    };

    println!(
        "\nCurrent test_recall contribution from path_mentions: {:.1}%",
        frac_already
    );
    println!(
        "UPPER BOUND additional from fuzzy symbol-name: +{:.1}% ({} new)",
        frac_new, new_caught_by_fuzzy
    );
    println!("Still missed after fuzzy: {:.1}%", frac_missed);
    println!(
        "Fuzzy noise: {} false-positive test files added",
        noise_fuzzy_additions
    );
    println!(
        "Signal:noise ratio = {:.2} (higher = better)",
        signal_to_noise
    );
    println!("\nGate for M2-07 impl: UPPER BOUND >= +4% test_recall AND signal:noise >= 1.0");
    let implement_ok = frac_new >= 4.0 && signal_to_noise >= 1.0;
    println!(
        "Recommendation: {}",
        if implement_ok {
            "IMPLEMENT M2-07"
        } else {
            "SKIP M2-07 — not worth the code surface"
        }
    );
}
