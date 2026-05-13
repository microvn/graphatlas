//! Audit C1 — dump engine's top hub files vs oracle's top churn files.
//! Marks each as test/prod via common::is_test_path.

use ga_index::Store;
use ga_query::common::is_test_path;
use ga_query::hubs::{hubs, HubsEdgeTypes, HubsRequest};
use ga_query::indexer::build_index;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn main() {
    let fixture: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: dump_hub_files <fixture>");
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache, &fixture).unwrap();
    build_index(&store, &fixture).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache, &fixture).unwrap();

    // Engine top hubs (default mode = Fix-A's all-edge query).
    let resp = hubs(
        &store,
        &HubsRequest {
            top_n: 200,
            symbol: None,
            file: None,
            edge_types: HubsEdgeTypes::Default,
        },
    )
    .unwrap();

    // Project symbols → max degree per file.
    let mut file_deg: HashMap<String, u32> = HashMap::new();
    for h in &resp.hubs {
        let total = h.in_degree + h.out_degree;
        let cur = file_deg.entry(h.file.clone()).or_insert(0);
        if total > *cur {
            *cur = total;
        }
    }
    let mut engine_files: Vec<(String, u32)> = file_deg.into_iter().collect();
    engine_files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Oracle: git log --name-only HEAD-anchored 12-month window.
    let head_ts = Command::new("git")
        .args(["-C", fixture.to_str().unwrap(), "log", "-1", "--format=%ct"])
        .output()
        .unwrap();
    let head_ct: i64 = String::from_utf8_lossy(&head_ts.stdout)
        .trim()
        .parse()
        .unwrap();
    let since = head_ct - 365 * 24 * 3600;
    let log = Command::new("git")
        .args([
            "-C",
            fixture.to_str().unwrap(),
            "log",
            &format!("--since=@{since}"),
            &format!("--before=@{head_ct}"),
            "--name-only",
            "--pretty=format:",
        ])
        .output()
        .unwrap();
    let mut oracle: HashMap<String, u32> = HashMap::new();
    for line in String::from_utf8_lossy(&log.stdout).lines() {
        let p = line.trim();
        if p.is_empty() || p.ends_with(".png") || p.ends_with(".jpg") || p.ends_with(".gif") {
            continue;
        }
        *oracle.entry(p.to_string()).or_insert(0) += 1;
    }
    let mut oracle_files: Vec<(String, u32)> = oracle.into_iter().collect();
    oracle_files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    println!("=== {} ===", fixture.display());
    println!("\n-- ENGINE top 20 (with [TEST] marker) --");
    for (f, d) in engine_files.iter().take(20) {
        let marker = if is_test_path(f) {
            "[TEST] "
        } else {
            "       "
        };
        println!("  {marker}{f:<70} deg={d}");
    }
    let total_engine = engine_files.len();
    let test_engine = engine_files.iter().filter(|(f, _)| is_test_path(f)).count();
    println!(
        "  ... engine total = {total_engine}, test files = {test_engine} ({:.0}%)",
        100.0 * test_engine as f64 / total_engine.max(1) as f64
    );

    println!("\n-- ORACLE top 20 (with [TEST] marker) --");
    for (f, c) in oracle_files.iter().take(20) {
        let marker = if is_test_path(f) {
            "[TEST] "
        } else {
            "       "
        };
        println!("  {marker}{f:<70} touches={c}");
    }
    let total_oracle = oracle_files.len();
    let test_oracle = oracle_files.iter().filter(|(f, _)| is_test_path(f)).count();
    println!(
        "  ... oracle total = {total_oracle}, test files = {test_oracle} ({:.0}%)",
        100.0 * test_oracle as f64 / total_oracle.max(1) as f64
    );

    // Compute Spearman ρ on intersection — once including tests, once excluding.
    let engine_pos: HashMap<&String, usize> = engine_files
        .iter()
        .enumerate()
        .map(|(i, (f, _))| (f, i))
        .collect();
    let oracle_pos: HashMap<&String, usize> = oracle_files
        .iter()
        .enumerate()
        .map(|(i, (f, _))| (f, i))
        .collect();

    let common_all: Vec<&String> = engine_pos
        .keys()
        .filter(|f| oracle_pos.contains_key(*f))
        .copied()
        .collect();
    let pairs_all: Vec<(usize, usize)> = common_all
        .iter()
        .map(|f| (oracle_pos[f], engine_pos[f]))
        .collect();
    let rho_all = spearman(&pairs_all);

    let common_prod: Vec<&String> = common_all
        .iter()
        .filter(|f| !is_test_path(f))
        .copied()
        .collect();
    let pairs_prod: Vec<(usize, usize)> = common_prod
        .iter()
        .map(|f| (oracle_pos[f], engine_pos[f]))
        .collect();
    let rho_prod = spearman(&pairs_prod);

    println!(
        "\n  ρ (all common files):  {rho_all:+.3}  (n={})",
        pairs_all.len()
    );
    println!(
        "  ρ (production-only):   {rho_prod:+.3}  (n={})  [drop {} test files]",
        pairs_prod.len(),
        pairs_all.len() - pairs_prod.len()
    );
}

fn spearman(pairs: &[(usize, usize)]) -> f64 {
    let n = pairs.len();
    if n < 2 {
        return 0.0;
    }
    let xs: Vec<f64> = to_rank(&pairs.iter().map(|p| p.0).collect::<Vec<_>>());
    let ys: Vec<f64> = to_rank(&pairs.iter().map(|p| p.1).collect::<Vec<_>>());
    let mx = xs.iter().sum::<f64>() / n as f64;
    let my = ys.iter().sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut dx2 = 0.0;
    let mut dy2 = 0.0;
    for i in 0..n {
        let dx = xs[i] - mx;
        let dy = ys[i] - my;
        num += dx * dy;
        dx2 += dx * dx;
        dy2 += dy * dy;
    }
    if dx2 == 0.0 || dy2 == 0.0 {
        return 0.0;
    }
    num / (dx2.sqrt() * dy2.sqrt())
}

fn to_rank(vs: &[usize]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..vs.len()).collect();
    order.sort_by_key(|&i| vs[i]);
    let mut rk = vec![0.0; vs.len()];
    for (r, &i) in order.iter().enumerate() {
        rk[i] = r as f64;
    }
    rk
}
