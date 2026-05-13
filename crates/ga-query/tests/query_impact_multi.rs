//! Tools S-006 cluster C8 — AS-013 multi-file union via `changed_files`.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn run_files(store: &Store, files: Vec<String>) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            changed_files: Some(files),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn multi_file_unions_symbols_across_three_files() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // 3 files each defining + calling a symbol.
    write(
        &repo.join("a.py"),
        "def a_fn(): pass\n\ndef a_caller():\n    a_fn()\n",
    );
    write(
        &repo.join("b.py"),
        "def b_fn(): pass\n\ndef b_caller():\n    b_fn()\n",
    );
    write(
        &repo.join("c.py"),
        "def c_fn(): pass\n\ndef c_caller():\n    c_fn()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["a.py".into(), "b.py".into(), "c.py".into()]);
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    for p in ["a.py", "b.py", "c.py"] {
        assert!(paths.contains(&p.to_string()), "missing {p}: {paths:?}");
    }
}

#[test]
fn multi_file_dedupes_shared_impacted_file_keeps_min_depth() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Both a.py and b.py have a symbol; the seed file for each contributes
    // itself at depth 0. If we pass both, each file is still depth 0 (seed).
    write(
        &repo.join("a.py"),
        "def shared(): pass\n\ndef a_local():\n    shared()\n",
    );
    write(
        &repo.join("b.py"),
        "def shared(): pass\n\ndef b_local():\n    shared()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["a.py".into(), "b.py".into()]);
    // Each file appears once despite being touched by both inputs.
    let a_count = resp
        .impacted_files
        .iter()
        .filter(|f| f.path == "a.py")
        .count();
    let b_count = resp
        .impacted_files
        .iter()
        .filter(|f| f.path == "b.py")
        .count();
    assert_eq!(a_count, 1);
    assert_eq!(b_count, 1);
    // Min-depth kept — a.py surfaces as seed (depth 0) at least once.
    let a_depth = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "a.py")
        .unwrap()
        .depth;
    assert_eq!(a_depth, 0);
}

#[test]
fn multi_file_break_points_cover_both_source_files() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\n\ndef a_caller():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\n\ndef b_caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["a.py".into(), "b.py".into()]);
    let files: Vec<String> = resp.break_points.iter().map(|bp| bp.file.clone()).collect();
    // Both files should appear as sources of a break point.
    assert!(files.contains(&"a.py".to_string()), "{files:?}");
    assert!(files.contains(&"b.py".to_string()), "{files:?}");
}

#[test]
fn multi_file_invalid_path_silently_skipped() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def a_fn(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // "evil'path.py" has a single quote — must be skipped without breaking.
    let resp = run_files(&store, vec!["evil'path.py".into(), "a.py".into()]);
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&"a.py".to_string()));
    // Nothing weird surfaced from the evil path.
    assert!(!paths.iter().any(|p| p.contains('\'')));
}

#[test]
fn multi_file_unknown_path_contributes_nothing() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def a_fn(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["not_indexed.py".into()]);
    assert!(resp.impacted_files.is_empty());
    assert!(resp.break_points.is_empty());
}

#[test]
fn multi_file_all_whitespace_paths_yield_empty_response() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def a_fn(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["   ".into(), "\t".into(), "".into()]);
    assert!(resp.impacted_files.is_empty());
}

#[test]
fn multi_file_risk_populated_when_signals_exist() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_files(&store, vec!["a.py".into()]);
    // Break points exist (caller→target) with no test file → risk > 0.
    assert!(
        resp.risk.score > 0.0,
        "expected positive risk for uncovered call site: {:?}",
        resp.risk
    );
}

#[test]
fn multi_file_deterministic_order() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("z.py"),
        "def z_fn(): pass\n\ndef z_caller():\n    z_fn()\n",
    );
    write(
        &repo.join("a.py"),
        "def a_fn(): pass\n\ndef a_caller():\n    a_fn()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // Input order reversed, output must still sort by (depth, path).
    let resp = run_files(&store, vec!["z.py".into(), "a.py".into()]);
    let paths: Vec<String> = resp
        .impacted_files
        .iter()
        .filter(|f| f.depth == 0)
        .map(|f| f.path.clone())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn multi_file_respects_max_depth() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // 3-link REFERENCES chain. max_depth=1 → only immediate reached.
    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef beta():\n    m = {'k': alpha}\n    return m\n",
    );
    write(
        &repo.join("c.py"),
        "from b import beta\n\ndef gamma():\n    m = {'k': beta}\n    return m\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            changed_files: Some(vec!["a.py".into()]),
            max_depth: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    let paths: Vec<String> = resp.impacted_files.iter().map(|f| f.path.clone()).collect();
    assert!(paths.contains(&"a.py".to_string()));
    assert!(paths.contains(&"b.py".to_string()));
    assert!(
        !paths.contains(&"c.py".to_string()),
        "max_depth=1 excludes 2-hop neighbor: {paths:?}"
    );
}
