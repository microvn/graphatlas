//! S-006 — Hrn-static rule (`ga_rename_safety` GT).
//!
//! Per spec:
//! - AS-017: per-target def_kind = "unique" (1 def file) | "polymorphic" (≥2 def files).
//! - AS-018.T1: expected_sites from `ga_parser::extract_calls` + `extract_references`,
//!   not via `ga_query::callers`.
//! - AS-018.T2: expected_blockers from line-local string-literal scan; multi-line
//!   string limitation listed in policy_bias.

use ga_bench::gt_gen::hrn_static::HrnStatic;
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

fn task_for<'a>(tasks: &'a [GeneratedTask], target: &str, file: &str) -> &'a GeneratedTask {
    tasks
        .iter()
        .find(|t| {
            t.query.get("target").and_then(|v| v.as_str()) == Some(target)
                && t.query.get("file").and_then(|v| v.as_str()) == Some(file)
        })
        .unwrap_or_else(|| panic!("no task for ({target}, {file})"))
}

#[test]
fn as_017_t1_single_def_yields_unique_tier() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("util.py"), "def renameable():\n    return 1\n");
    write(
        &repo.join("driver.py"),
        "from util import renameable\n\ndef driver():\n    return renameable()\n",
    );
    let (store, _t) = empty_store(repo);
    let tasks = HrnStatic.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "renameable", "util.py");
    assert_eq!(
        t.query.get("def_kind").and_then(|v| v.as_str()),
        Some("unique"),
        "single def file → def_kind=unique; got: {}",
        t.query
    );
}

#[test]
fn as_017_t2_two_defs_yields_polymorphic_tier() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a.py"), "def shared():\n    return 1\n");
    write(&repo.join("b.py"), "def shared():\n    return 2\n");
    let (store, _t) = empty_store(repo);
    let tasks = HrnStatic.scan(&store, repo).unwrap();
    // Both a.py::shared and b.py::shared must be marked polymorphic.
    for f in ["a.py", "b.py"] {
        let t = task_for(&tasks, "shared", f);
        assert_eq!(
            t.query.get("def_kind").and_then(|v| v.as_str()),
            Some("polymorphic"),
            "{f}::shared has a homonym → polymorphic; got: {}",
            t.query
        );
    }
}

#[test]
fn as_018_t1_expected_sites_collected_from_calls() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("u.py"), "def alpha():\n    return 1\n");
    write(
        &repo.join("d.py"),
        "from u import alpha\n\ndef driver():\n    return alpha()\n",
    );
    let (store, _t) = empty_store(repo);
    let tasks = HrnStatic.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "alpha", "u.py");
    let sites = t
        .query
        .get("expected_sites")
        .and_then(|v| v.as_array())
        .unwrap();
    let has_call_site = sites
        .iter()
        .any(|s| s.get("file").and_then(|v| v.as_str()) == Some("d.py"));
    assert!(
        has_call_site,
        "expected_sites must include call site in d.py; got: {:?}",
        sites
    );
}

#[test]
fn as_018_t2_string_literal_blocker_recorded() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("u.py"), "def beta():\n    return 1\n");
    write(&repo.join("dyn.py"), "method_name = \"beta\"\n");
    let (store, _t) = empty_store(repo);
    let tasks = HrnStatic.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "beta", "u.py");
    let blockers = t
        .query
        .get("expected_blockers")
        .and_then(|v| v.as_array())
        .unwrap();
    let has_string_blocker = blockers.iter().any(|b| {
        b.get("file").and_then(|v| v.as_str()) == Some("dyn.py")
            && b.get("reason").and_then(|v| v.as_str()) == Some("string_literal")
    });
    assert!(
        has_string_blocker,
        "blocker for `beta` must include dyn.py string_literal; got: {:?}",
        blockers
    );
}

#[test]
fn as_018_t2_def_line_not_recorded_as_string_blocker() {
    // The def line itself contains the symbol name in code form (not a
    // string literal). Must not be flagged as a blocker.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("u.py"), "def gamma():\n    return 1\n");
    let (store, _t) = empty_store(repo);
    let tasks = HrnStatic.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "gamma", "u.py");
    let blockers = t
        .query
        .get("expected_blockers")
        .and_then(|v| v.as_array())
        .unwrap();
    let false_positive = blockers
        .iter()
        .any(|b| b.get("file").and_then(|v| v.as_str()) == Some("u.py"));
    assert!(
        !false_positive,
        "code-form `gamma` on def line must NOT be recorded as string_literal blocker; got: {:?}",
        blockers
    );
}

#[test]
fn as_018_policy_bias_documents_multi_line_string_limitation() {
    let rule = HrnStatic;
    let bias = rule.policy_bias().to_lowercase();
    assert!(
        bias.contains("multi-line") || bias.contains("multiline"),
        "policy_bias must note multi-line string blocker limitation; got: {bias}"
    );
}

#[test]
fn as_018_id_and_uc_match_spec() {
    let r = HrnStatic;
    assert_eq!(r.id(), "Hrn-static");
    assert_eq!(r.uc(), "rename_safety");
}
