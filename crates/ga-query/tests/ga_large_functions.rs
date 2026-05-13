//! `ga_large_functions` — line-span filter smoke tests.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::large_functions::{large_functions, LargeFunctionsRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (tmp, cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn flags_only_functions_above_min_lines() {
    let (_tmp, cache, repo) = setup();
    // `big` spans 12 lines; `small` spans 2 lines.
    let mut src = String::from("def small():\n    return 1\n\n");
    src.push_str("def big():\n");
    for i in 0..10 {
        src.push_str(&format!("    x{i} = {i}\n"));
    }
    src.push_str("    return 0\n");
    write(&repo.join("a.py"), &src);

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = large_functions(
        &store,
        &LargeFunctionsRequest {
            min_lines: 8,
            limit: 10,
            ..Default::default()
        },
    )
    .expect("large_functions ok");

    let names: Vec<&str> = resp.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"big"), "big spans 12 lines; got {names:?}");
    assert!(
        !names.contains(&"small"),
        "small spans 2 lines; must NOT trip min_lines=8; got {names:?}"
    );
}

#[test]
fn line_count_invariant_holds() {
    let (_tmp, cache, repo) = setup();
    let mut src = String::from("def fat():\n");
    for i in 0..30 {
        src.push_str(&format!("    a{i} = {i}\n"));
    }
    src.push_str("    return 0\n");
    write(&repo.join("a.py"), &src);

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = large_functions(
        &store,
        &LargeFunctionsRequest {
            min_lines: 1,
            ..Default::default()
        },
    )
    .expect("large_functions ok");

    let fat = resp
        .functions
        .iter()
        .find(|f| f.name == "fat")
        .expect("fat present");
    assert_eq!(
        fat.line_count,
        fat.line_end - fat.line + 1,
        "line_count = line_end - line + 1 invariant"
    );
    assert!(fat.line_count >= 30, "fat spans 30+ lines; got {fat:?}");
}

#[test]
fn kind_filter_restricts() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "class Big:\n    pass\n    pass\n    pass\n    pass\n    pass\n\n\
         def big_fn():\n    a=1\n    b=2\n    c=3\n    d=4\n    return 0\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = large_functions(
        &store,
        &LargeFunctionsRequest {
            min_lines: 3,
            kind: Some("class".to_string()),
            ..Default::default()
        },
    )
    .expect("large_functions ok");

    for f in &resp.functions {
        assert_eq!(f.kind, "class", "kind filter must be exact; got {f:?}");
    }
    let names: Vec<&str> = resp.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Big"), "Big class should appear");
    assert!(
        !names.contains(&"big_fn"),
        "big_fn (function) must be filtered out"
    );
}

#[test]
fn limit_caps_response() {
    let (_tmp, cache, repo) = setup();
    let mut src = String::new();
    for i in 0..20 {
        src.push_str(&format!("def f{i}():\n"));
        for _ in 0..6 {
            src.push_str("    pass\n");
        }
        src.push('\n');
    }
    write(&repo.join("a.py"), &src);

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = large_functions(
        &store,
        &LargeFunctionsRequest {
            min_lines: 5,
            limit: 5,
            ..Default::default()
        },
    )
    .expect("large_functions ok");

    assert_eq!(resp.functions.len(), 5, "limit=5 must cap to 5");
    assert!(resp.meta.truncated, "20+ candidates → truncated");
    assert!(resp.meta.total_matches >= 20);
}
