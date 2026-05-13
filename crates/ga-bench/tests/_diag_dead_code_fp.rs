//! Audit dead_code false positives on django: ga marks dead, Hd-ast says live.
//! Sample 20 cases + dump source line so we can see the pattern.
//! Run: cargo test -p ga-query --test _diag_dead_code_fp -- --ignored --nocapture

use ga_index::Store;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[test]
#[ignore]
fn diag_django_dead_code_fp() {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let cache = repo_root.join(".graphatlas-bench-cache/m3-dead_code/django/ga");
    let fixture = repo_root.join("benches/fixtures/django");
    let _ = std::fs::remove_dir_all(&cache);
    let store = Store::open_with_root(&cache, &fixture).expect("open store");
    ga_query::indexer::build_index(&store, &fixture).expect("build_index");

    // Run ga_dead_code
    let resp =
        ga_query::dead_code::dead_code(&store, &ga_query::dead_code::DeadCodeRequest::default())
            .expect("dead_code");
    let actual: BTreeSet<(String, String)> = resp
        .dead
        .iter()
        .map(|d| (d.file.clone(), d.symbol.clone()))
        .collect();
    println!("ga_dead_code returned {} dead entries", actual.len());

    // Run Hd-ast bench rule
    use ga_bench::gt_gen::hd_ast::HdAst;
    use ga_bench::gt_gen::GtRule;
    let tasks = HdAst.scan(&store, &fixture).expect("hd_ast scan");
    let mut expected_dead: BTreeSet<(String, String)> = BTreeSet::new();
    let mut bench_says_live: BTreeSet<(String, String)> = BTreeSet::new();
    for t in &tasks {
        let name = t
            .query
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file = t
            .query
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || file.is_empty() {
            continue;
        }
        let is_dead = t
            .query
            .get("expected_dead")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_dead {
            expected_dead.insert((file.clone(), name.clone()));
        } else {
            bench_says_live.insert((file, name));
        }
    }
    println!(
        "Hd-ast: {} expected_dead, {} expected_live",
        expected_dead.len(),
        bench_says_live.len()
    );

    // FP = ga.dead but not in expected_dead = bench thinks LIVE
    let fps: Vec<(String, String)> = actual
        .iter()
        .filter(|p| !expected_dead.contains(p))
        .cloned()
        .collect();
    println!(
        "\n=== {} false positives (ga: dead, bench: live) — first 25 ===\n",
        fps.len()
    );

    let mut samples = fps.clone();
    samples.sort();
    for (file, name) in samples.iter().take(25) {
        // Read source line from def
        let in_bench = bench_says_live.contains(&(file.clone(), name.clone()));
        // Try grep for `def <name>` or `fn <name>` in file
        let abs = fixture.join(file);
        let mut matched_line = String::new();
        if let Ok(text) = std::fs::read_to_string(&abs) {
            for (i, line) in text.lines().enumerate() {
                if line.contains(&format!("def {name}"))
                    || line.contains(&format!("fn {name}"))
                    || line.contains(&format!("class {name}"))
                {
                    matched_line = format!("L{}: {}", i + 1, line.trim());
                    break;
                }
            }
        }
        println!(
            "  {}::{}\n    in_bench_live={} | source: {}",
            file, name, in_bench, matched_line
        );
    }

    // Pattern detection: count by suffix / prefix / kind
    let mut by_suffix: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, name) in &fps {
        let suffix = if name.starts_with("test_") {
            "test_*"
        } else if name.starts_with('_') {
            "_private"
        } else if name.starts_with("__") {
            "__dunder"
        } else if name == "__init__" || name == "__str__" || name == "__repr__" {
            "__init__/__str__/__repr__"
        } else {
            "other"
        };
        *by_suffix.entry(suffix.to_string()).or_insert(0) += 1;
    }
    println!("\n=== Pattern distribution of FP names ===");
    let mut by_suffix: Vec<_> = by_suffix.into_iter().collect();
    by_suffix.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, v) in by_suffix {
        println!("  {k:30} : {v}");
    }
}
