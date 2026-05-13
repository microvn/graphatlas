//! Bench P1 — Retriever trait end-to-end via run_uc_with. Small hand-built
//! fixture + GT + a MockRetriever that pins actuals. Keeps the integration
//! surface fast — no submodule dependency.

use ga_bench::retriever::Retriever;
use ga_bench::retrievers::{GaRetriever, RipgrepRetriever};
use ga_bench::runner::{run_uc_with, RunOpts};
use ga_bench::BenchError;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup_fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(
        &repo.join("auth.py"),
        "def check_password(u,p): return True\n\ndef login_view(u,p):\n    return check_password(u,p)\n",
    );
    write(&repo.join("views.py"), "def index(): return 'ok'\n");

    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{
            "schema_version": 1,
            "uc": "callers",
            "fixture": "tmp",
            "tasks": [
                {"task_id":"check_password","query":{"symbol":"check_password"},"expected":["login_view"]}
            ]
        }"#,
    )
    .unwrap();

    (tmp, repo, gt_path)
}

struct MockRetriever {
    called: bool,
}

impl Retriever for MockRetriever {
    fn name(&self) -> &str {
        "mock"
    }
    fn query(&mut self, _uc: &str, _q: &Value) -> Result<Vec<String>, BenchError> {
        self.called = true;
        Ok(vec!["login_view".to_string()])
    }
}

#[test]
fn run_uc_with_drives_custom_retriever_list() {
    let (tmp, repo, gt_path) = setup_fixture();
    let out_md = tmp.path().join("out.md");

    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(MockRetriever { called: false })];
    let lb = run_uc_with(
        RunOpts {
            uc: "callers".to_string(),
            fixture_dir: repo,
            gt_path,
            cache_root: tmp.path().join(".graphatlas"),
            out_md: out_md.clone(),
        },
        retrievers,
    )
    .expect("run_uc_with must succeed");

    assert_eq!(lb.entries.len(), 1);
    assert_eq!(lb.entries[0].retriever, "mock");
    // Mock returned the correct answer → F1 should be 1.0 for this 1-task GT.
    assert_eq!(lb.entries[0].f1, 1.0);
    assert_eq!(lb.entries[0].pass_rate, 1.0);
    assert!(out_md.is_file());
}

#[test]
fn ga_retriever_via_trait_hits_real_graph() {
    let (tmp, repo, gt_path) = setup_fixture();
    let out_md = tmp.path().join("out.md");

    let retrievers: Vec<Box<dyn Retriever>> =
        vec![Box::new(GaRetriever::new(tmp.path().join(".graphatlas")))];
    let lb = run_uc_with(
        RunOpts {
            uc: "callers".to_string(),
            fixture_dir: repo,
            gt_path,
            cache_root: tmp.path().join(".graphatlas-unused"), // GaRetriever owns its own
            out_md,
        },
        retrievers,
    )
    .expect("GA via trait");

    let ga = lb.entries.iter().find(|e| e.retriever == "ga").unwrap();
    assert_eq!(
        ga.f1, 1.0,
        "GA should solve trivial-fixture callers exactly"
    );
}

#[test]
fn ripgrep_retriever_empty_on_callers_uc() {
    // ripgrep has no structural resolution → returns [] for callers,
    // pass_rate drops to 0 on a 1-task GT (task failed). This is the
    // correct behavior per Bench-C3 (honest baseline).
    let (tmp, repo, gt_path) = setup_fixture();
    let out_md = tmp.path().join("out.md");

    let retrievers: Vec<Box<dyn Retriever>> = vec![Box::new(RipgrepRetriever::new())];
    let lb = run_uc_with(
        RunOpts {
            uc: "callers".to_string(),
            fixture_dir: repo,
            gt_path,
            cache_root: tmp.path().join(".graphatlas"),
            out_md,
        },
        retrievers,
    )
    .expect("ripgrep via trait");

    let rg = lb
        .entries
        .iter()
        .find(|e| e.retriever == "ripgrep")
        .unwrap();
    // F1 = 0 because rg returns empty, expected is non-empty.
    assert_eq!(rg.f1, 0.0);
    assert_eq!(rg.pass_rate, 0.0);
}
