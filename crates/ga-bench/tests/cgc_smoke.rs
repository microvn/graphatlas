//! Bench P4-C1 — CgcRetriever smoke. Gated on `cgc` on PATH:
//!   - tool missing: emit "SKIPPED: cgc not on PATH" and pass
//!   - tool present: run the bench end-to-end, assert the leaderboard row
//!     exists and retriever ran without crashing
//!
//! Not a correctness test of CGC itself — just proves our subprocess +
//! MCP plumbing doesn't regress when a real tool shows up.

mod common;

use common::{setup_mini_fixture, tool_present};
use ga_bench::retriever::Retriever;
use ga_bench::retrievers::CgcRetriever;
use ga_bench::runner::{run_uc_with, RunOpts};
use tempfile::TempDir;

#[test]
fn cgc_retriever_renders_leaderboard_row_when_tool_present() {
    if !tool_present("cgc") {
        eprintln!("SKIPPED: cgc not on PATH — install `cgc` to exercise this path");
        return;
    }
    let (tmp, repo, gt_path) = setup_mini_fixture();
    let out_dir = TempDir::new().unwrap();
    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(CgcRetriever::new())];
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
    .expect("run_uc_with");
    let entry = lb
        .entries
        .iter()
        .find(|e| e.retriever == "codegraphcontext")
        .expect("cgc entry must be present in leaderboard");
    // Real numbers depend on the installed cgc version + whether pre-index
    // succeeded — don't hard-pin pass_rate. Just prove the entry reached the
    // scorer (pass_rate + f1 are valid floats in [0, 1]).
    assert!(entry.pass_rate >= 0.0 && entry.pass_rate <= 1.0);
    assert!(entry.f1 >= 0.0 && entry.f1 <= 1.0);
    eprintln!(
        "cgc smoke: f1={:.3} pass_rate={:.3} p95={}ms",
        entry.f1, entry.pass_rate, entry.p95_latency_ms
    );
}
