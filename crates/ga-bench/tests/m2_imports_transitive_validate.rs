//! File-IMPORTS transitive validate harness (Lỗi 3 from POC comparison).
//!
//! POC's post-loop adds `File<-[:IMPORTS]-File` traversal — files that
//! IMPORT the target at depth 2. M2 has IMPORTS edges in schema but
//! never traverses them in `bfs_from_symbol`. This harness simulates
//! adding those files to impacted_files and measures signal:noise on GT
//! expected_files.

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

/// Files that IMPORT the given target file.
fn importers_of(conn: &lbug::Connection<'_>, target: &str) -> HashSet<String> {
    let safe = target.replace('\'', "''");
    let cypher = format!(
        "MATCH (importer:File)-[:IMPORTS]->(target:File) \
         WHERE target.path = '{safe}' RETURN DISTINCT importer.path"
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
fn m2_imports_transitive_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-imports-validate");

    if !gt_path.exists() {
        eprintln!("[SKIP] GT missing");
        return;
    }
    let gt = match M2GroundTruth::load(&gt_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[SKIP] GT load: {e}");
            return;
        }
    };
    println!("Loaded {} tasks", gt.tasks.len());

    let dev: Vec<&ga_bench::M2Task> = gt.tasks.iter().filter(|t| t.split == Split::Dev).collect();

    let mut by_repo: std::collections::HashMap<String, Vec<&ga_bench::M2Task>> =
        std::collections::HashMap::new();
    for t in &dev {
        by_repo.entry(t.repo.clone()).or_default().push(t);
    }

    let mut total_expected = 0usize;
    let mut already_in = 0usize;
    let mut new_via_imports = 0usize;
    let mut still_missed = 0usize;
    let mut total_noise = 0usize;
    let mut per_repo: Vec<(String, usize, usize, usize, usize, usize)> = Vec::new();

    for (repo, tasks) in &by_repo {
        let fixture = fixtures_root.join(repo);
        if !fixture.exists() {
            continue;
        }
        let cache = cache_root.join(repo);
        let _ = std::fs::remove_dir_all(&cache);
        let store = match Store::open_with_root(&cache, &fixture) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[SKIP {}] {e}", repo);
                continue;
            }
        };
        if build_index(&store, &fixture).is_err() {
            continue;
        }
        let conn = store.connection().unwrap();

        let mut r_total = 0;
        let mut r_already = 0;
        let mut r_new = 0;
        let mut r_missed = 0;
        let mut r_noise = 0;

        for task in tasks {
            let expected: HashSet<&String> = task.expected_files.iter().collect();
            let current = current_impacted(&store, &task.seed_symbol);

            // Simulate POC: add files that IMPORT seed_file + files that
            // IMPORT any currently-impacted file.
            let mut imports_signal: HashSet<String> = HashSet::new();
            imports_signal.extend(importers_of(&conn, &task.seed_file));
            for impacted in &current {
                imports_signal.extend(importers_of(&conn, impacted));
            }

            for ef in &task.expected_files {
                r_total += 1;
                if current.contains(ef.as_str()) {
                    r_already += 1;
                } else if imports_signal.contains(ef.as_str()) {
                    r_new += 1;
                } else {
                    r_missed += 1;
                }
            }
            for s in &imports_signal {
                if expected.contains(s) {
                    continue;
                }
                if current.contains(s) {
                    continue;
                }
                r_noise += 1;
            }
        }
        per_repo.push((repo.clone(), r_total, r_already, r_new, r_missed, r_noise));
        total_expected += r_total;
        already_in += r_already;
        new_via_imports += r_new;
        still_missed += r_missed;
        total_noise += r_noise;
    }

    println!("\n=== FILE-IMPORTS TRANSITIVE VALIDATION (Lỗi 3) ===");
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "repo", "total", "already", "new", "missed", "noise"
    );
    for (r, t, a, n, m, ns) in &per_repo {
        println!("{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}", r, t, a, n, m, ns);
    }
    println!(
        "{:<12}{:>8}{:>10}{:>10}{:>10}{:>10}",
        "TOTAL", total_expected, already_in, new_via_imports, still_missed, total_noise
    );

    let frac_new = if total_expected > 0 {
        new_via_imports as f64 / total_expected as f64 * 100.0
    } else {
        0.0
    };
    let sig_noise = if total_noise == 0 {
        f64::INFINITY
    } else {
        new_via_imports as f64 / total_noise as f64
    };

    println!(
        "\nUPPER BOUND added: +{:.1}% ({} new)",
        frac_new, new_via_imports
    );
    println!(
        "still missed: {:.1}%",
        if total_expected > 0 {
            still_missed as f64 / total_expected as f64 * 100.0
        } else {
            0.0
        }
    );
    println!("noise: {}, signal:noise = {:.2}", total_noise, sig_noise);
    println!("\nGate: UPPER BOUND >= +4% AND signal:noise >= 1.0");
    let go = frac_new >= 4.0 && sig_noise >= 1.0;
    println!("Recommendation: {}", if go { "IMPLEMENT" } else { "SKIP" });
}
