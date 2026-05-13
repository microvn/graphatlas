//! M3 minimal_context rule `Hmc-gitmine` — reads
//! `benches/uc-impact/ground-truth.json` (M1+M2 git-mining dataset)
//! and emits per-task GeneratedTask for the M3 minimal_context UC.
//!
//! Replaces the archived `Hmc-budget` rule (LLM-generated `tasks-v6`
//! dataset, low accuracy).

use ga_bench::gt_gen::hmc_gitmine::{HmcGitmine, Split};
use ga_bench::gt_gen::GtRule;
use ga_index::Store;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn open_store(fixture_dir: &PathBuf) -> Store {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    // GRAPHATLAS_CACHE_DIR rejects modes >0700; tempfile defaults to 0755.
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_with_root(tmp.path(), fixture_dir).expect("open store");
    std::mem::forget(tmp); // keep the dir alive past the function return
    store
}

fn skip_if_no_django() -> Option<(PathBuf, Store)> {
    let root = workspace_root();
    let fixture = root.join("benches/fixtures/django");
    if !fixture.is_dir() {
        eprintln!("[SKIP] django submodule not initialised");
        return None;
    }
    let store = open_store(&fixture);
    Some((fixture, store))
}

#[test]
fn scan_returns_django_test_split_only_by_default() {
    let Some((fixture, store)) = skip_if_no_django() else {
        return;
    };
    let rule = HmcGitmine::for_fixture("django");
    let tasks = rule.scan(&store, &fixture).expect("scan");
    assert!(!tasks.is_empty(), "expected django split:test tasks, got 0");
    for t in &tasks {
        assert_eq!(
            t.query.get("repo").and_then(|v| v.as_str()),
            Some("django"),
            "task {} not filtered by repo",
            t.task_id
        );
        assert_eq!(
            t.query.get("__split").and_then(|v| v.as_str()),
            Some("test"),
            "task {} leaked split != test (default scoring split)",
            t.task_id
        );
    }
}

#[test]
fn scan_with_split_dev_returns_dev_only() {
    let Some((fixture, store)) = skip_if_no_django() else {
        return;
    };
    let rule = HmcGitmine::for_fixture("django").with_split(Split::Dev);
    let tasks = rule.scan(&store, &fixture).expect("scan");
    for t in &tasks {
        assert_eq!(
            t.query.get("__split").and_then(|v| v.as_str()),
            Some("dev"),
            "task {} should be dev split when with_split(Dev) is set",
            t.task_id
        );
    }
}

#[test]
fn scan_returns_no_tasks_for_unknown_fixture() {
    let Some((fixture, store)) = skip_if_no_django() else {
        return;
    };
    let rule = HmcGitmine::for_fixture("definitely-not-a-real-repo");
    let tasks = rule.scan(&store, &fixture).expect("scan");
    assert!(tasks.is_empty(), "unknown fixture must yield 0 tasks");
}

#[test]
fn scan_emits_required_query_fields() {
    let Some((fixture, store)) = skip_if_no_django() else {
        return;
    };
    let rule = HmcGitmine::for_fixture("django");
    let tasks = rule.scan(&store, &fixture).expect("scan");
    let t = tasks.first().expect("at least one task");

    // Required: symbol (input to ga_minimal_context), repo, base_commit
    // (for fixture pinning), seed_file (for resolver-fail diagnostic).
    assert!(t.query.get("symbol").and_then(|v| v.as_str()).is_some());
    assert!(t.query.get("repo").and_then(|v| v.as_str()).is_some());
    assert!(t
        .query
        .get("__base_commit")
        .and_then(|v| v.as_str())
        .is_some());
    assert!(t
        .query
        .get("__seed_file")
        .and_then(|v| v.as_str())
        .is_some());
    assert!(t
        .query
        .get("__expected_tests")
        .and_then(|v| v.as_array())
        .is_some());

    // expected = expected_files (file-level GT)
    assert!(!t.expected.is_empty(), "expected_files must be non-empty");
}

#[test]
fn scan_is_deterministic_ordered_by_task_id() {
    let Some((fixture, store)) = skip_if_no_django() else {
        return;
    };
    let rule = HmcGitmine::for_fixture("django");
    let tasks = rule.scan(&store, &fixture).expect("scan");
    let ids: Vec<&str> = tasks.iter().map(|t| t.task_id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        ids, sorted,
        "tasks must be sorted by task_id for reproducibility"
    );
}

#[test]
fn rule_metadata_marks_dataset_source() {
    let rule = HmcGitmine::default();
    assert_eq!(rule.id(), "Hmc-gitmine");
    assert_eq!(rule.uc(), "minimal_context");
    let bias = rule.policy_bias();
    assert!(
        bias.contains("git-mining") || bias.contains("ground-truth.json"),
        "policy_bias must name the dataset source: {bias}"
    );
}
