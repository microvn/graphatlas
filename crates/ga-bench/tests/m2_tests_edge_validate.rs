//! File-TESTS edge validate harness (POC-parity spike).
//!
//! POC schema has `File-[TESTS]->File` edges emitted at index time when a
//! test file's symbols CALLS production-file symbols. POC's BFS then
//! surfaces test files in `impacted_files` directly (depth 1 for target,
//! depth 2 for other impacted files).
//!
//! M2 doesn't have this edge — tests live in the separate `affected_tests`
//! field and contribute to test_recall (not precision/completeness). This
//! harness simulates the POC semantic WITHOUT a schema change by
//! aggregating existing `TESTED_BY` + `DEFINES` edges to file level, then
//! measures on dev corpus:
//!
//!   - How many GT `expected_files` are test files M2 currently MISSES
//!     (absent from impacted_files) but the POC pattern would catch?
//!   - How many extra test files would the pattern surface that are NOT
//!     in GT (noise)?
//!
//! Gate: `signal:noise >= 1.0` AND `new-catch >= +4% of expected_files` →
//! worth coding as a real EXP. Same pattern as `m2_07_recall_harness`.

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

/// Simulate POC's file-TESTS edge query using existing TESTED_BY edges
/// aggregated to file level. Returns set of test files that TEST the
/// given production file.
fn tests_linked_to_file(conn: &lbug::Connection<'_>, prod_file: &str) -> HashSet<String> {
    let safe = prod_file.replace('\'', "''");
    let cypher = format!(
        "MATCH (pf:File)-[:DEFINES]->(ps:Symbol)-[:TESTED_BY]->(ts:Symbol)<-[:DEFINES]-(tf:File) \
         WHERE pf.path = '{safe}' RETURN DISTINCT tf.path"
    );
    let mut out = HashSet::new();
    let Ok(rs) = conn.query(&cypher) else {
        return out;
    };
    for row in rs {
        if let Some(lbug::Value::String(p)) = row.into_iter().next() {
            out.insert(p);
        }
    }
    out
}

/// Current M2 impacted_files set for the task's seed.
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

#[test]
#[ignore]
fn m2_tests_edge_recall_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-tests-edge-validate");

    if !gt_path.exists() {
        eprintln!("[SKIP] ground-truth.json missing");
        return;
    }
    let gt = match M2GroundTruth::load(&gt_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[SKIP] GT load: {e}");
            return;
        }
    };
    println!(
        "Loaded {} tasks (schema v{})",
        gt.tasks.len(),
        gt.schema_version
    );

    let dev: Vec<&ga_bench::M2Task> = gt.tasks.iter().filter(|t| t.split == Split::Dev).collect();
    println!("Dev split: {} tasks", dev.len());

    let mut by_repo: std::collections::HashMap<String, Vec<&ga_bench::M2Task>> =
        std::collections::HashMap::new();
    for t in &dev {
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }

    let mut total_expected = 0usize;
    let mut already_in_impacted = 0usize;
    let mut new_via_tests_edge = 0usize;
    let mut still_missed = 0usize;
    let mut total_noise_added = 0usize;

    let mut per_repo: Vec<(String, usize, usize, usize, usize, usize)> = Vec::new();
    // (repo, total_gt, already, new, missed, noise)

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
                eprintln!("[SKIP repo={}] open: {e}", repo);
                continue;
            }
        };
        if let Err(e) = build_index(&store, &fixture) {
            eprintln!("[SKIP repo={}] build: {e}", repo);
            continue;
        }
        let conn = store.connection().unwrap();

        let mut r_total = 0usize;
        let mut r_already = 0usize;
        let mut r_new = 0usize;
        let mut r_missed = 0usize;
        let mut r_noise = 0usize;

        for task in tasks {
            let expected: HashSet<&String> = task.expected_files.iter().collect();
            let current = current_impacted(&store, &task.seed_symbol);

            // POC semantic — test files TESTS-linked to seed_file, plus
            // test files TESTS-linked to each currently-impacted file
            // (POC's depth-2 pattern).
            let mut tests_edge_set: HashSet<String> = HashSet::new();
            tests_edge_set.extend(tests_linked_to_file(&conn, &task.seed_file));
            for impacted in &current {
                tests_edge_set.extend(tests_linked_to_file(&conn, impacted));
            }

            for expected_file in &task.expected_files {
                r_total += 1;
                if current.contains(expected_file.as_str()) {
                    r_already += 1;
                } else if tests_edge_set.contains(expected_file.as_str()) {
                    r_new += 1;
                } else {
                    r_missed += 1;
                }
            }

            // Noise: files in tests_edge_set that are NOT expected AND
            // NOT already in M2 current impacted (to avoid double-counting
            // files already surfaced by BFS).
            for t in &tests_edge_set {
                if expected.contains(t) {
                    continue;
                }
                if current.contains(t) {
                    continue;
                }
                r_noise += 1;
            }
        }

        per_repo.push((repo.clone(), r_total, r_already, r_new, r_missed, r_noise));
        total_expected += r_total;
        already_in_impacted += r_already;
        new_via_tests_edge += r_new;
        still_missed += r_missed;
        total_noise_added += r_noise;
    }

    println!("\n=== FILE-TESTS EDGE RECALL VALIDATION (Lỗi 1) ===");
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "repo", "total", "already", "new", "missed", "noise"
    );
    for (r, t, a, n, m, ns) in &per_repo {
        println!("{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}", r, t, a, n, m, ns);
    }
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "TOTAL",
        total_expected,
        already_in_impacted,
        new_via_tests_edge,
        still_missed,
        total_noise_added
    );

    let frac_already = if total_expected > 0 {
        already_in_impacted as f64 / total_expected as f64 * 100.0
    } else {
        0.0
    };
    let frac_new = if total_expected > 0 {
        new_via_tests_edge as f64 / total_expected as f64 * 100.0
    } else {
        0.0
    };
    let frac_missed = if total_expected > 0 {
        still_missed as f64 / total_expected as f64 * 100.0
    } else {
        0.0
    };
    let sig_noise = if total_noise_added == 0 {
        f64::INFINITY
    } else {
        new_via_tests_edge as f64 / total_noise_added as f64
    };

    println!("\ncurrent impacted_files coverage: {:.1}%", frac_already);
    println!(
        "UPPER BOUND added by file-TESTS edge: +{:.1}% ({} new)",
        frac_new, new_via_tests_edge
    );
    println!("still missed after TESTS edge: {:.1}%", frac_missed);
    println!(
        "TESTS-edge noise: {} files not in GT expected_files",
        total_noise_added
    );
    println!("signal:noise = {:.2} (gate: >= 1.0)", sig_noise);
    println!("\nGate: UPPER BOUND >= +4% AND signal:noise >= 1.0");
    let go = frac_new >= 4.0 && sig_noise >= 1.0;
    println!(
        "Recommendation: {}",
        if go {
            "IMPLEMENT file-TESTS edge in schema + BFS"
        } else {
            "SKIP — not worth the schema/indexer change"
        }
    );
}
