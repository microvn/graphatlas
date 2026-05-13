//! S-007 — Ha-import-edge rule (`ga_architecture` GT).
//!
//! Per spec:
//! - AS-019.T1: edge GT counts file pairs (module_a, module_b).
//! - AS-019.T2: marker-based module set tracked separately as diagnostic.
//! - AS-019.T3: primary metric — Spearman if utility, else F1 fallback;
//!   policy_bias documents the metric choice.
//!
//! AS-020 (TAUTOLOGY-SUSPECT row) is a runner concern (m3_runner aggregates
//! Spearman/F1 across fixtures); the rule emits raw GT only.

use ga_bench::gt_gen::ha_import_edge::HaImportEdge;
use ga_bench::gt_gen::{GeneratedTask, GtRule};
use ga_index::Store;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn empty_store(repo: &Path) -> (Store, TempDir) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache, repo).unwrap();
    (store, tmp)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn edge_tasks(tasks: &[GeneratedTask]) -> Vec<&GeneratedTask> {
    tasks
        .iter()
        .filter(|t| t.query.get("kind").and_then(|v| v.as_str()) == Some("edge"))
        .collect()
}

fn edge_for<'a>(tasks: &'a [GeneratedTask], module_a: &str, module_b: &str) -> &'a GeneratedTask {
    tasks
        .iter()
        .find(|t| {
            t.query.get("kind").and_then(|v| v.as_str()) == Some("edge")
                && t.query.get("module_a").and_then(|v| v.as_str()) == Some(module_a)
                && t.query.get("module_b").and_then(|v| v.as_str()) == Some(module_b)
        })
        .unwrap_or_else(|| panic!("no edge ({module_a} → {module_b})"))
}

#[test]
fn as_019_t1_edge_counts_file_pairs() {
    // Two marker-based modules a/ and b/ (each has __init__.py).
    // a/x.py imports from b/. → edge a→b, file_pair_count=1.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("b/__init__.py"), "");
    write(
        &repo.join("a/x.py"),
        "from b.foo import F\n\ndef use():\n    return F\n",
    );
    write(&repo.join("a/y.py"), "def standalone():\n    return 1\n");
    write(&repo.join("b/foo.py"), "F = 1\n");

    let (store, _t) = empty_store(repo);
    let tasks = HaImportEdge.scan(&store, repo).unwrap();
    let edge = edge_for(&tasks, "a", "b");
    assert_eq!(
        edge.query.get("file_pair_count").and_then(|v| v.as_u64()),
        Some(1),
        "edge a→b should count exactly 1 importing file (a/x.py); got: {}",
        edge.query
    );
}

#[test]
fn as_019_t1_two_files_importing_same_module_count_separately() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("b/__init__.py"), "");
    write(&repo.join("a/x.py"), "from b.foo import F\n");
    write(&repo.join("a/y.py"), "from b.bar import G\n");
    write(&repo.join("b/foo.py"), "F = 1\n");
    write(&repo.join("b/bar.py"), "G = 2\n");
    let (store, _t) = empty_store(repo);
    let tasks = HaImportEdge.scan(&store, repo).unwrap();
    let edge = edge_for(&tasks, "a", "b");
    assert_eq!(
        edge.query.get("file_pair_count").and_then(|v| v.as_u64()),
        Some(2),
        "two distinct importing files in `a` → file_pair_count=2"
    );
}

#[test]
fn as_019_t1_self_edges_excluded() {
    // a/x.py imports from a/y → same module; not a cross-module edge.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("a/x.py"), "from a.y import G\n");
    write(&repo.join("a/y.py"), "G = 1\n");
    let (store, _t) = empty_store(repo);
    let tasks = HaImportEdge.scan(&store, repo).unwrap();
    let edges = edge_tasks(&tasks);
    let self_edge = edges.iter().any(|e| {
        e.query.get("module_a").and_then(|v| v.as_str())
            == e.query.get("module_b").and_then(|v| v.as_str())
    });
    assert!(
        !self_edge,
        "intra-module imports must not produce edges; tasks: {:?}",
        edges.iter().map(|e| e.query.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn as_019_t2_module_set_emitted_as_diagnostic() {
    // Marker-based module: dir with __init__.py is a module.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("pkg/__init__.py"), "");
    write(&repo.join("pkg/lib.py"), "def f():\n    return 1\n");
    let (store, _t) = empty_store(repo);
    let tasks = HaImportEdge.scan(&store, repo).unwrap();
    let module_tasks: Vec<&GeneratedTask> = tasks
        .iter()
        .filter(|t| t.query.get("kind").and_then(|v| v.as_str()) == Some("module"))
        .collect();
    let pkg_module = module_tasks
        .iter()
        .find(|t| t.query.get("module").and_then(|v| v.as_str()) == Some("pkg"))
        .unwrap_or_else(|| panic!("module `pkg` (marked by __init__.py) must be in GT"));
    let files = pkg_module
        .query
        .get("files")
        .and_then(|v| v.as_array())
        .expect("module entry must list files");
    assert!(
        files.iter().any(|f| f.as_str() == Some("pkg/lib.py")),
        "module pkg should list pkg/lib.py; got: {:?}",
        files
    );
}

#[test]
fn as_019_t3_policy_bias_documents_metric_choice() {
    let rule = HaImportEdge;
    let bias = rule.policy_bias().to_lowercase();
    assert!(
        bias.contains("spearman") || bias.contains("f1"),
        "policy_bias must name the primary metric (Spearman or F1); got: {bias}"
    );
    assert!(
        bias.contains("tautolog") || bias.contains("marker"),
        "policy_bias must surface the marker-tautology caveat; got: {bias}"
    );
}

#[test]
fn as_019_id_and_uc_match_spec() {
    let r = HaImportEdge;
    assert_eq!(r.id(), "Ha-import-edge");
    assert_eq!(r.uc(), "architecture");
}

#[test]
fn as_019_empty_fixture_returns_empty_tasks_no_panic() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    let (store, _t) = empty_store(repo);
    let tasks = HaImportEdge.scan(&store, repo).unwrap();
    assert!(
        tasks.is_empty(),
        "empty fixture → no tasks; got {} tasks",
        tasks.len()
    );
}
