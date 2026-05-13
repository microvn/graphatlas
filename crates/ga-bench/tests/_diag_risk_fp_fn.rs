//! Diagnostic: per-file dim breakdown for FN/FP files in `ga_risk` bench.
//!
//! For each fixture, runs the full bench scoring path and prints a table of
//! (file, GT label, predicted, dim values). Goal: identify whether the
//! 7-8 FN files on regex/tokio are caused by:
//!   (a) bug_correlation undercount (pipeline bug)
//!   (b) test_gap drowning out bug signal (formula-vs-GT mismatch)
//!   (c) blast/churn pulling score down (formula tradeoff)
//!
//! Run:
//!   cargo test -p ga-bench --test _diag_risk_fp_fn -- --ignored --nocapture
//! Optional fixture filter:
//!   GA_DIAG_FIXTURE=regex cargo test -p ga-bench --test _diag_risk_fp_fn -- --ignored --nocapture

use ga_bench::gt_gen::hr_text::{resolve_head_sha, HrText};
use ga_bench::gt_gen::GtRule;
use ga_query::blame::GitLogMiner;
use ga_query::risk::{risk, RiskRequest};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const FIXTURES: &[&str] = &["axum", "gin", "nest", "regex", "tokio"];
const RISKY_CUTOFF: f32 = 0.30;
const MAX_FILES_TO_SCORE: usize = 20;

fn diag_one(fixture: &str) {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let fixture_dir = repo_root.join("benches/fixtures").join(fixture);
    if !fixture_dir.is_dir() {
        eprintln!("[SKIP] {fixture}: not initialised");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let cache = std::env::temp_dir().join(format!("ga-diag-risk-{fixture}"));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).unwrap();

    let gt_store_dir = cache.join("gt-probe");
    std::fs::create_dir_all(&gt_store_dir).unwrap();
    std::fs::set_permissions(&gt_store_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let gt_store = ga_index::Store::open_with_root(&gt_store_dir, &fixture_dir).unwrap();

    let rule = HrText;
    let tasks = rule.scan(&gt_store, &fixture_dir).unwrap();
    if tasks.is_empty() {
        eprintln!("[SKIP] {fixture}: empty GT");
        return;
    }

    // Same sampling as m3_risk.rs: half risky, half non-risky, cap 20.
    let mut expected_risky_files: BTreeSet<String> = BTreeSet::new();
    let mut bug_density: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut all_files: Vec<String> = Vec::new();
    for t in &tasks {
        let file = t
            .query
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if file.is_empty() {
            continue;
        }
        all_files.push(file.clone());
        let cc = t
            .query
            .get("commit_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let bc = t
            .query
            .get("bug_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        bug_density.insert(file.clone(), (cc, bc));
        if t.query
            .get("expected_risky")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            expected_risky_files.insert(file);
        }
    }
    let risky_cap = MAX_FILES_TO_SCORE / 2;
    let non_risky_cap = MAX_FILES_TO_SCORE - risky_cap;
    let risky_sample: Vec<String> = expected_risky_files
        .iter()
        .take(risky_cap)
        .cloned()
        .collect();
    let non_risky_sample: Vec<String> = all_files
        .iter()
        .filter(|f| !expected_risky_files.contains(*f))
        .take(non_risky_cap)
        .cloned()
        .collect();
    let to_score: Vec<String> = risky_sample.into_iter().chain(non_risky_sample).collect();

    // Build index once, then per-file ga_risk for per-dim breakdown.
    let store_dir = cache.join("ga");
    std::fs::create_dir_all(&store_dir).unwrap();
    std::fs::set_permissions(&store_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = ga_index::Store::open_with_root(&store_dir, &fixture_dir).unwrap();
    ga_query::indexer::build_index(&store, &fixture_dir).unwrap();
    let miner = GitLogMiner::new(&fixture_dir);

    println!("\n══════════════════════════════════════════════════════════════════════");
    println!("FIXTURE: {fixture}");
    println!("══════════════════════════════════════════════════════════════════════");
    println!(
        "{:<55} {:>3} {:>4} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}",
        "file", "GT", "pred", "comp", "test", "blst", "chrn", "bug", "bd%"
    );
    println!("{:-<99}", "");

    let mut fn_scores: Vec<(String, [f32; 4], f32)> = Vec::new();
    let mut fp_scores: Vec<(String, [f32; 4], f32)> = Vec::new();
    let mut tp_scores: Vec<(String, [f32; 4], f32)> = Vec::new();

    let head_sha = resolve_head_sha(&fixture_dir);
    for file in &to_score {
        let mut req = RiskRequest::for_changed_files(vec![file.clone()]);
        if !head_sha.is_empty() {
            req.anchor_ref = Some(head_sha.clone());
        }
        let resp = match risk(&store, &miner, &req) {
            Ok(r) => r,
            Err(e) => {
                println!("{file:<55} ERR {e}");
                continue;
            }
        };
        let dims = resp.meta.per_dim;
        let score = resp.score;
        let gt_risky = expected_risky_files.contains(file);
        let predicted = score >= RISKY_CUTOFF;
        let label = match (gt_risky, predicted) {
            (true, true) => "TP",
            (true, false) => "FN",
            (false, true) => "FP",
            (false, false) => "TN",
        };
        let (cc, bc) = bug_density.get(file).copied().unwrap_or((0, 0));
        let bd_pct = if cc == 0 {
            0.0
        } else {
            bc as f64 / cc as f64 * 100.0
        };
        println!(
            "{:<55} {:>3} {:>4} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.0}",
            truncate(file, 55),
            label,
            if predicted { "Y" } else { "N" },
            score,
            dims.test_gap,
            dims.blast_radius,
            dims.blame_churn,
            dims.bug_correlation,
            bd_pct
        );
        let dim_arr = [
            dims.test_gap,
            dims.blast_radius,
            dims.blame_churn,
            dims.bug_correlation,
        ];
        match label {
            "FN" => fn_scores.push((file.clone(), dim_arr, score)),
            "FP" => fp_scores.push((file.clone(), dim_arr, score)),
            "TP" => tp_scores.push((file.clone(), dim_arr, score)),
            _ => {}
        }
    }

    fn mean(vals: &[f32]) -> f32 {
        if vals.is_empty() {
            0.0
        } else {
            vals.iter().sum::<f32>() / vals.len() as f32
        }
    }
    fn dim_means(rows: &[(String, [f32; 4], f32)]) -> [f32; 4] {
        let mut m = [0.0; 4];
        for d in 0..4 {
            let v: Vec<f32> = rows.iter().map(|r| r.1[d]).collect();
            m[d] = mean(&v);
        }
        m
    }
    let fn_m = dim_means(&fn_scores);
    let fp_m = dim_means(&fp_scores);
    let tp_m = dim_means(&tp_scores);
    println!("\n── Mean dim values ──");
    println!("                test  blast churn bug   composite");
    println!(
        "FN (n={:>2}):     {:.2}  {:.2}  {:.2}  {:.2}  {:.2}",
        fn_scores.len(),
        fn_m[0],
        fn_m[1],
        fn_m[2],
        fn_m[3],
        mean(&fn_scores.iter().map(|r| r.2).collect::<Vec<_>>())
    );
    println!(
        "FP (n={:>2}):     {:.2}  {:.2}  {:.2}  {:.2}  {:.2}",
        fp_scores.len(),
        fp_m[0],
        fp_m[1],
        fp_m[2],
        fp_m[3],
        mean(&fp_scores.iter().map(|r| r.2).collect::<Vec<_>>())
    );
    println!(
        "TP (n={:>2}):     {:.2}  {:.2}  {:.2}  {:.2}  {:.2}",
        tp_scores.len(),
        tp_m[0],
        tp_m[1],
        tp_m[2],
        tp_m[3],
        mean(&tp_scores.iter().map(|r| r.2).collect::<Vec<_>>())
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - (max - 3)..])
    }
}

#[test]
#[ignore]
fn diag_risk_fp_fn() {
    let filter = std::env::var("GA_DIAG_FIXTURE").ok();
    for fx in FIXTURES {
        if let Some(ref want) = filter {
            if fx != want {
                continue;
            }
        }
        diag_one(fx);
    }
}
