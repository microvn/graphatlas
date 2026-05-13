//! EXTENDS validate harness (Lỗi 2b from POC comparison).
//!
//! POC's BFS includes EXTENDS edges alongside CALLS+TESTED_BY. M2's BFS
//! uses CALLS+REFERENCES only. EXTENDS edge exists in M2 schema but is
//! never traversed. This harness simulates adding 1-hop EXTENDS expansion
//! from the seed symbol + from each symbol in currently-impacted files.

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

/// 1-hop EXTENDS expansion (both directions). Returns files of symbols
/// that either EXTEND or are EXTENDED BY any symbol named `seed`.
fn extends_files(conn: &lbug::Connection<'_>, seed: &str) -> HashSet<String> {
    let safe = seed.replace('\'', "''");
    let mut out = HashSet::new();
    // Direction 1: seed extends something (child -> parent).
    let q1 = format!(
        "MATCH (s:Symbol)-[:EXTENDS]->(p:Symbol) \
         WHERE s.name = '{safe}' AND p.kind <> 'external' RETURN DISTINCT p.file"
    );
    if let Ok(rs) = conn.query(&q1) {
        for row in rs {
            if let Some(lbug::Value::String(p)) = row.into_iter().next() {
                out.insert(p);
            }
        }
    }
    // Direction 2: something extends seed (child -> seed).
    let q2 = format!(
        "MATCH (c:Symbol)-[:EXTENDS]->(s:Symbol) \
         WHERE s.name = '{safe}' AND c.kind <> 'external' RETURN DISTINCT c.file"
    );
    if let Ok(rs) = conn.query(&q2) {
        for row in rs {
            if let Some(lbug::Value::String(p)) = row.into_iter().next() {
                out.insert(p);
            }
        }
    }
    out
}

/// Collect all symbol names defined in `file_path`. Used to expand
/// EXTENDS from every symbol in currently-impacted files.
fn symbols_in_file(conn: &lbug::Connection<'_>, path: &str) -> HashSet<String> {
    let safe = path.replace('\'', "''");
    let cypher = format!(
        "MATCH (f:File {{path: '{safe}'}})-[:DEFINES]->(s:Symbol) \
         WHERE s.kind IN ['class','struct','trait','interface'] RETURN DISTINCT s.name"
    );
    let mut out = HashSet::new();
    let Ok(rs) = conn.query(&cypher) else {
        return out;
    };
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.insert(s);
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
fn m2_extends_harness() {
    let root = workspace_root();
    let gt_path = root.join("benches/uc-impact/ground-truth.json");
    let fixtures_root = root.join("benches/fixtures");
    let cache_root = root.join(".graphatlas-bench-cache/m2-extends-validate");

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

    // 3-pool classification (2026-04-24 reframe):
    //   hits_expected     = new file ∈ expected_files (lifts composite)
    //   hits_should_touch = new file ∈ should_touch_files (lifts adj_prec + blast_radius)
    //   hits_neither      = noise (not in either GT pool)
    let mut total_expected_files = 0usize;
    let mut total_should_touch_files = 0usize;
    let mut already_expected = 0usize; // expected already in current GA
    let mut new_hits_expected = 0usize; // expected NOT in current, ADDED by EXTENDS
    let mut already_should_touch = 0usize; // should_touch already in current GA
    let mut new_hits_should_touch = 0usize; // should_touch NOT in current, ADDED
    let mut noise = 0usize; // EXTENDS output NOT in either GT pool
    let mut per_repo: Vec<(String, usize, usize, usize, usize, usize, usize, usize)> = Vec::new();

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
        let conn = store.connection().unwrap();

        let mut r_exp = 0;
        let mut r_stf = 0;
        let mut r_already_exp = 0;
        let mut r_new_exp = 0;
        let mut r_already_stf = 0;
        let mut r_new_stf = 0;
        let mut r_noise = 0;

        for task in tasks {
            let expected: HashSet<&String> = task.expected_files.iter().collect();
            let should_touch: HashSet<&String> = task.should_touch_files.iter().collect();
            let current = current_impacted(&store, &task.seed_symbol);

            // Simulate EXTENDS expansion:
            // 1. Direct from seed symbol
            let mut ext_signal: HashSet<String> = HashSet::new();
            ext_signal.extend(extends_files(&conn, &task.seed_symbol));
            // 2. From each class/struct in currently-impacted files
            for imp_file in &current {
                for sym in symbols_in_file(&conn, imp_file) {
                    ext_signal.extend(extends_files(&conn, &sym));
                }
            }

            // Classify GT files (expected pool)
            for ef in &task.expected_files {
                r_exp += 1;
                if current.contains(ef.as_str()) {
                    r_already_exp += 1;
                } else if ext_signal.contains(ef.as_str()) {
                    r_new_exp += 1;
                }
            }
            // Classify GT files (should_touch pool)
            for sf in &task.should_touch_files {
                r_stf += 1;
                if current.contains(sf.as_str()) {
                    r_already_stf += 1;
                } else if ext_signal.contains(sf.as_str()) {
                    r_new_stf += 1;
                }
            }
            // Noise = EXTENDS output not in either GT pool AND not already in current
            for s in &ext_signal {
                if expected.contains(s) || should_touch.contains(s) {
                    continue;
                }
                if current.contains(s) {
                    continue;
                }
                r_noise += 1;
            }
        }
        per_repo.push((
            repo.clone(),
            r_exp,
            r_stf,
            r_already_exp,
            r_new_exp,
            r_already_stf,
            r_new_stf,
            r_noise,
        ));
        total_expected_files += r_exp;
        total_should_touch_files += r_stf;
        already_expected += r_already_exp;
        new_hits_expected += r_new_exp;
        already_should_touch += r_already_stf;
        new_hits_should_touch += r_new_stf;
        noise += r_noise;
    }

    println!("\n=== H-M3 EXTENDS EDGE VALIDATION (3-POOL, 2026-04-24) ===");
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
        "TOTAL",
        total_expected_files,
        total_should_touch_files,
        already_expected,
        new_hits_expected,
        already_should_touch,
        new_hits_should_touch,
        noise
    );

    let new_total = new_hits_expected + new_hits_should_touch;
    let sig_noise = if noise == 0 {
        f64::INFINITY
    } else {
        new_total as f64 / noise as f64
    };
    let pct_new_exp = if total_expected_files > 0 {
        new_hits_expected as f64 / total_expected_files as f64 * 100.0
    } else {
        0.0
    };
    let pct_new_stf = if total_should_touch_files > 0 {
        new_hits_should_touch as f64 / total_should_touch_files as f64 * 100.0
    } else {
        0.0
    };

    println!(
        "\nCOMPOSITE lift:      +{:.1}% expected_files ({} new)",
        pct_new_exp, new_hits_expected
    );
    println!(
        "BLAST_RADIUS lift:   +{:.1}% should_touch_files ({} new)",
        pct_new_stf, new_hits_should_touch
    );
    println!(
        "SIGNAL:NOISE         {} new / {} noise = {:.2}",
        new_total, noise, sig_noise
    );
    let go = sig_noise >= 0.2 && new_total >= 3; // gate per IMPACT_MACRO_STRATEGY: 1:5 sig:noise
    println!(
        "GATE (s/n ≥ 0.2, new ≥ 3): {}",
        if go { "✅ IMPLEMENT" } else { "❌ SKIP" }
    );
}
