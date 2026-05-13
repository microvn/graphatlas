//! Bench P4-C1 — CmRetriever smoke. Gated on `codebase-memory-mcp` on PATH.

mod common;

use common::{setup_mini_fixture, tool_present};
use ga_bench::retriever::Retriever;
use ga_bench::retrievers::CmRetriever;
use ga_bench::runner::{run_uc_with, RunOpts};
use tempfile::TempDir;

#[test]
fn cm_retriever_renders_leaderboard_row_when_tool_present() {
    if !tool_present("codebase-memory-mcp") {
        eprintln!("SKIPPED: codebase-memory-mcp not on PATH");
        return;
    }
    let (tmp, repo, gt_path) = setup_mini_fixture();
    let out_dir = TempDir::new().unwrap();
    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(CmRetriever::new())];
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
        .find(|e| e.retriever == "codebase-memory")
        .expect("cm entry must be present");
    assert!(entry.pass_rate >= 0.0 && entry.pass_rate <= 1.0);
    eprintln!(
        "cm smoke: f1={:.3} pass_rate={:.3} p95={}ms",
        entry.f1, entry.pass_rate, entry.p95_latency_ms
    );
}
