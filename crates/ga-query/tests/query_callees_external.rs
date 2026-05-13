//! Tools S-002 cluster B — external callees (stdlib/third-party/unresolved)
//! surface with `external: true`. Spec: AS-004 Data clause `hashlib.sha256`.

use ga_index::Store;
use ga_query::{callees, callers, indexer::build_index};
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

#[test]
fn external_callee_flagged_in_response() {
    // `authenticate` calls local check_password (resolved) + external
    // hashlib.sha256 (not defined in repo). Expect both callees; only the
    // second flagged external: true.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("auth.py"),
        "def check_password(): pass\n\ndef authenticate():\n    check_password()\n    hashlib.sha256()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "authenticate", None).unwrap();
    let names: Vec<&str> = resp.callees.iter().map(|c| c.symbol.as_str()).collect();
    assert!(names.contains(&"check_password"), "{names:?}");
    assert!(names.contains(&"sha256"), "{names:?}");

    let internal = resp
        .callees
        .iter()
        .find(|c| c.symbol == "check_password")
        .expect("internal callee");
    assert!(!internal.external, "within-repo should not be external");

    let external = resp
        .callees
        .iter()
        .find(|c| c.symbol == "sha256")
        .expect("external callee");
    assert!(external.external, "stdlib callee must flag external: true");
}

#[test]
fn external_callees_deduped_across_repo() {
    // Two separate callers both call the same external sha256. Graph should
    // carry one external Symbol node, not two — verified by probing callers.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "def fa():\n    sha256()\n");
    write(&repo.join("b.py"), "def fb():\n    sha256()\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // Callers of the external sha256 should include both fa and fb — only
    // possible if there's a single shared external node.
    let resp = callers(&store, "sha256", None).unwrap();
    let mut names: Vec<String> = resp.callers.iter().map(|c| c.symbol.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["fa".to_string(), "fb".to_string()]);
}

#[test]
fn external_does_not_inflate_def_count() {
    // An external named `helper` plus ONE real def of `helper` in a.py →
    // def_count remains 1 (externals excluded), so callers of helper get
    // confidence 1.0 (unambiguous), not 0.6 (polymorphic).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def helper(): pass\ndef caller_a():\n    helper()\n",
    );
    // b.py invokes a same-named function that has no local def → external.
    write(&repo.join("b.py"), "def caller_b():\n    helper()\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // callers of the real helper (a.py). Only caller_a resolves within-file
    // to the real helper; caller_b's call targets the external helper node.
    let resp = callers(&store, "helper", Some("a.py")).unwrap();
    let exact: Vec<_> = resp
        .callers
        .iter()
        .filter(|c| (c.confidence - 1.0).abs() < 1e-6)
        .collect();
    assert!(
        exact.iter().any(|c| c.symbol == "caller_a"),
        "caller_a should be exact-match 1.0 (single real def): {:?}",
        resp.callers
    );
}

#[test]
fn callers_of_external_symbol_returns_real_caller() {
    // ga_callers against an external name surfaces the real caller(s).
    // Useful for "who uses hashlib.sha256 in this repo?" introspection.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("auth.py"), "def authenticate():\n    sha256()\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "sha256", None).unwrap();
    assert_eq!(resp.callers.len(), 1);
    assert_eq!(resp.callers[0].symbol, "authenticate");
}
