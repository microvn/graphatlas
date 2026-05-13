//! Per-task audit for regex + tokio: pin per task, show GT vs engine
//! response, classify noise (typo/grammar/fmt commit subject).
//!
//! Run:
//!   cargo test -p ga-query --test _audit_rust_repos -- --ignored --nocapture
use ga_index::Store;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git_checkout(repo: &Path, sha: &str) -> bool {
    Command::new("git")
        .args(["-C", &repo.display().to_string(), "checkout", "-q", sha])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_head(repo: &Path) -> String {
    String::from_utf8(
        Command::new("git")
            .args(["-C", &repo.display().to_string(), "rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}

fn classify(subject: &str) -> &'static str {
    let s = subject.to_lowercase();
    if s.contains("typo") || s.contains("grammar") || s.starts_with("chore: fix") {
        "NOISE/typo"
    } else if s.starts_with("fmt:") || s.contains("cargo fmt") || s.contains("run 'cargo fmt") {
        "NOISE/fmt"
    } else if s.starts_with("docs:") || s.contains("readme") {
        "NOISE/docs"
    } else {
        "real-fix"
    }
}

fn audit_repo(repo: &str) {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let fixture = repo_root.join("benches/fixtures").join(repo);
    if !fixture.is_dir() {
        eprintln!("[SKIP] {repo} not init'd");
        return;
    }

    let gt: Value = serde_json::from_slice(
        &std::fs::read(repo_root.join("benches/uc-impact/ground-truth.json")).unwrap(),
    )
    .unwrap();

    let mut tasks: Vec<&Value> = gt["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["repo"].as_str() == Some(repo) && t["split"].as_str() == Some("test"))
        .collect();
    tasks.sort_by_key(|t| t["task_id"].as_str().unwrap_or("").to_string());

    let original = git_head(&fixture);
    let mut total_recall = 0.0;
    let mut count = 0;
    let mut noise_count = 0;

    println!("\n════════════════════════════════════════════════════════════════════");
    println!("REPO: {repo} ({} tasks)", tasks.len());
    println!("════════════════════════════════════════════════════════════════════\n");

    for t in &tasks {
        let id = t["task_id"].as_str().unwrap();
        let base = t["base_commit"].as_str().unwrap();
        let sym = t["seed_symbol"].as_str().unwrap();
        let seed_file = t["seed_file"].as_str().unwrap();
        let subject = t["subject"].as_str().unwrap_or("");
        let expected: Vec<&str> = t["expected_files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        let label = classify(subject);
        if label != "real-fix" {
            noise_count += 1;
        }

        if !git_checkout(&fixture, base) {
            println!("─── {id} [{label}] PIN FAIL ── {subject}\n");
            continue;
        }

        let cache = repo_root
            .join(".graphatlas-bench-cache/audit-rust")
            .join(repo)
            .join(id);
        let _ = std::fs::remove_dir_all(&cache);
        let store = match Store::open_with_root(&cache, &fixture) {
            Ok(s) => s,
            Err(e) => {
                println!("─── {id} STORE FAIL: {e}\n");
                continue;
            }
        };
        if ga_query::indexer::build_index(&store, &fixture).is_err() {
            println!("─── {id} INDEX FAIL\n");
            continue;
        }

        let req = ga_query::minimal_context::MinimalContextRequest::for_symbol_in_file(
            sym, seed_file, 2000,
        );
        let resp = ga_query::minimal_context::minimal_context(&store, &req);

        match resp {
            Ok(r) => {
                let actual: std::collections::BTreeSet<&str> =
                    r.symbols.iter().map(|s| s.file.as_str()).collect();
                let hits = expected.iter().filter(|f| actual.contains(*f)).count();
                let recall = hits as f64 / expected.len() as f64;
                total_recall += recall;
                count += 1;

                println!(
                    "─── {id} [{label}] sym={sym} n={} hits={}/{} recall={:.3}",
                    expected.len(),
                    hits,
                    expected.len(),
                    recall
                );
                println!("    SUBJ:  {subject}");
                println!("    SEED:  {seed_file}");
                if expected.len() > 1 {
                    println!("    EXPECTED:");
                    for f in &expected {
                        let mark = if actual.contains(f) { "  ✓" } else { "  ✗" };
                        println!("    {mark} {f}");
                    }
                }
                println!(
                    "    RETURNED ({} ctx): {}",
                    r.symbols.len(),
                    r.symbols
                        .iter()
                        .map(|s| format!("{:?}={}", s.reason, s.file))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                println!();
            }
            Err(e) => {
                println!("─── {id} [{label}] sym={sym} ERROR: {e}");
                println!("    SUBJ:  {subject}\n");
                count += 1;
            }
        }
    }

    git_checkout(&fixture, &original);

    let mean = if count == 0 {
        0.0
    } else {
        total_recall / count as f64
    };
    println!("════════════════════════════════════════════════════════════════════");
    println!(
        "{repo} SUMMARY: mean_recall={:.3} | tasks={count} | noise_subjects={noise_count}",
        mean
    );
    println!("════════════════════════════════════════════════════════════════════\n");
}

#[test]
#[ignore]
fn audit_axum() {
    audit_repo("axum");
}

#[test]
#[ignore]
fn audit_regex() {
    audit_repo("regex");
}

#[test]
#[ignore]
fn audit_tokio() {
    audit_repo("tokio");
}
