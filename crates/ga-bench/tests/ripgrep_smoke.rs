//! Bench P4-C1 — RipgrepRetriever smoke. Gated on `rg` binary (not a shell
//! alias) being callable via Command::new("rg"). Real-rg path asserts the
//! `importers` UC surfaces matching files via filename-stem grep.

mod common;

use ga_bench::retriever::Retriever;
use ga_bench::retrievers::RipgrepRetriever;
use ga_bench::runner::{run_uc_with, RunOpts};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn rg_binary_callable() -> bool {
    // Command::new uses exec + PATH, ignores shell functions/aliases.
    // If this returns a usable output, the retriever will succeed too.
    Command::new("rg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn ripgrep_importers_uc_returns_files_when_binary_present() {
    if !rg_binary_callable() {
        eprintln!("SKIPPED: rg binary not callable via Command::new (shell alias only?)");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("utils.py"), "def fmt(x): return str(x)\n");
    write(&repo.join("a.py"), "from utils import fmt\n");
    write(&repo.join("b.py"), "from utils import fmt\n");

    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{
            "schema_version": 1,
            "uc": "importers",
            "fixture": "rg-smoke",
            "tasks": [
                {"task_id":"utils_importers","query":{"file":"utils.py"},"expected":["a.py","b.py"]}
            ]
        }"#,
    )
    .unwrap();

    let out_dir = TempDir::new().unwrap();
    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(RipgrepRetriever::new())];
    let lb = run_uc_with(
        RunOpts {
            uc: "importers".to_string(),
            fixture_dir: repo,
            gt_path,
            cache_root: tmp.path().join(".graphatlas"),
            out_md: out_dir.path().join("out.md"),
        },
        retrievers,
    )
    .expect("run_uc_with");
    let rg = lb
        .entries
        .iter()
        .find(|e| e.retriever == "ripgrep")
        .unwrap();
    // Both a.py and b.py contain "utils" → recall should be ≥ 0.5.
    assert!(
        rg.recall >= 0.5,
        "ripgrep recall {:.3} too low for trivial filename-stem match",
        rg.recall
    );
}

#[test]
fn ripgrep_disabled_when_binary_missing() {
    // Regression guard for Bench-C7 graceful-disable: even when `rg` isn't
    // available, the retriever must produce a leaderboard row (pass_rate=0)
    // rather than aborting the run. We can't uninstall rg from the test
    // env, so we simulate by running callers UC where rg returns empty by
    // design — same failure mode from the scorer's perspective.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("m.py"), "def foo(): pass\n");
    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        json!({
            "schema_version": 1,
            "uc": "callers",
            "fixture": "rg-disabled",
            "tasks": [
                {"task_id":"foo","query":{"symbol":"foo"},"expected":["bar"]}
            ]
        })
        .to_string(),
    )
    .unwrap();

    let out_dir = TempDir::new().unwrap();
    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(RipgrepRetriever::new())];
    let lb = run_uc_with(
        RunOpts {
            uc: "callers".to_string(),
            fixture_dir: repo,
            gt_path,
            cache_root: tmp.path().join(".graphatlas"),
            out_md: out_dir.path().join("out.md"),
        },
        retrievers,
    )
    .expect("must not abort");
    let rg = lb
        .entries
        .iter()
        .find(|e| e.retriever == "ripgrep")
        .unwrap();
    // Row present, pass_rate 0 (rg can't answer callers UC).
    assert_eq!(rg.pass_rate, 0.0);
    assert_eq!(rg.f1, 0.0);
}
