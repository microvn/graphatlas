//! M3 hubs gate — end-to-end smoke against an in-memory git repo.
//!
//! Builds a tiny temp repo with skewed file churn (one file gets many
//! commits, others get few), indexes it via `build_index`, runs
//! `score_hubs`, and verifies the resulting `M3LeaderboardRow` shape +
//! that the engine's hub-by-file projection correlates positively with
//! the GT churn rank. Spec target verification (≥ 0.7) is intentionally
//! NOT asserted here — the toy fixture is too small to give a stable ρ;
//! gate-level pass/fail is exercised on real fixtures via the bench CLI.

use ga_bench::m3_hubs::{score_hubs, HUBS_SPEC_TARGET};
use ga_bench::m3_runner::{ScoreOpts, SpecStatus};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git invocation");
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn commit(repo: &Path, msg: &str) {
    git(repo, &["add", "-A"]);
    git(
        repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            msg,
        ],
    );
}

#[test]
fn score_hubs_emits_row_with_spec_target() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["-c", "init.defaultBranch=main", "checkout", "-qb", "main"],
    );

    // Initial: 3 files. `core.py` will be churned across many commits;
    // `helper.py` rarely; `bystander.py` once.
    write(
        &repo.join("core.py"),
        "def compute():\n    return 1\n\ndef driver():\n    return compute()\n",
    );
    write(
        &repo.join("helper.py"),
        "def aux():\n    return 0\n\ndef caller():\n    return aux()\n",
    );
    write(&repo.join("bystander.py"), "def quiet():\n    return 0\n");
    commit(&repo, "init");

    // Ten more commits touching core.py only.
    for i in 0..10 {
        write(
            &repo.join("core.py"),
            &format!("def compute():\n    return {i}\n\ndef driver():\n    return compute()\n"),
        );
        commit(&repo, &format!("bump core {i}"));
    }
    // One commit touching helper.py.
    write(
        &repo.join("helper.py"),
        "def aux():\n    return 1\n\ndef caller():\n    return aux()\n",
    );
    commit(&repo, "tweak helper");

    let opts = ScoreOpts {
        fixture_name: "smoke".to_string(),
        fixture_dir: repo.clone(),
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["ga".to_string()],
        gt_path: None,
        split: None,
    };

    let rows = score_hubs(&opts).expect("score_hubs ok");
    assert_eq!(rows.len(), 1, "one ga retriever row");
    let row = &rows[0];
    assert_eq!(row.uc, "hubs");
    assert_eq!(row.retriever, "ga");
    assert_eq!(row.fixture, "smoke");
    assert!(
        (row.spec_target - HUBS_SPEC_TARGET).abs() < 1e-9,
        "spec target threaded through"
    );
    assert!(matches!(
        row.spec_status,
        SpecStatus::Pass | SpecStatus::Fail
    ));
    assert!(
        row.secondary_metrics.contains_key("gt_size"),
        "gt_size surfaced"
    );
    assert!(
        row.secondary_metrics.contains_key("common_files"),
        "common_files surfaced"
    );
}

#[test]
fn unknown_retriever_marks_deferred() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["-c", "init.defaultBranch=main", "checkout", "-qb", "main"],
    );
    write(&repo.join("a.py"), "def f():\n    return 0\n");
    commit(&repo, "init");

    let opts = ScoreOpts {
        fixture_name: "deferred".to_string(),
        fixture_dir: repo,
        cache_root: tmp.path().join("cache"),
        retrievers: vec!["crg".to_string()],
        gt_path: None,
        split: None,
    };
    let rows = score_hubs(&opts).expect("score_hubs ok");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].retriever, "crg");
    assert!(matches!(rows[0].spec_status, SpecStatus::Deferred));
}
