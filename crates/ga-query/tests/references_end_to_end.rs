//! AS-019 — callers / callees surface REFERENCES edges alongside CALLS,
//! each entry tagged with `kind: Call | Reference`.

use ga_index::Store;
use ga_query::{callees, callers, indexer::build_index, CallKind};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

#[test]
fn ga_callers_returns_reference_kind_for_dispatch_map() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handlers.ts"),
        "export function handleUsers() { return 1; }\n",
    );
    write(
        &repo.join("routes.ts"),
        "import { handleUsers } from './handlers';\nexport function setup() {\n  const routes = { '/api/users': handleUsers };\n  return routes;\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "handleUsers", None).unwrap();
    assert!(
        resp.callers
            .iter()
            .any(|c| c.symbol == "setup" && c.kind == CallKind::Reference),
        "expected setup as Reference caller; got {:?}",
        resp.callers
    );
}

#[test]
fn ga_callers_includes_both_call_and_reference_kinds() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("lib.ts"),
        "export function target() { return 1; }\n",
    );
    write(
        &repo.join("both.ts"),
        "import { target } from './lib';\nexport function callsite() {\n  target();\n}\nexport function refsite() {\n  const m = { t: target };\n  return m;\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    let kinds: Vec<_> = resp
        .callers
        .iter()
        .map(|c| (c.symbol.clone(), c.kind))
        .collect();
    assert!(
        kinds
            .iter()
            .any(|(s, k)| s == "callsite" && *k == CallKind::Call),
        "missing Call kind for callsite: {kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(s, k)| s == "refsite" && *k == CallKind::Reference),
        "missing Reference kind for refsite: {kinds:?}"
    );
}

#[test]
fn ga_callees_returns_reference_kind_when_function_holds_callback() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.ts"),
        "export function ping() { return 'ok'; }\nexport function setup() {\n  const map = { p: ping };\n  return map;\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "setup", None).unwrap();
    assert!(
        resp.callees
            .iter()
            .any(|c| c.symbol == "ping" && c.kind == CallKind::Reference),
        "setup should have Reference callee ping; got {:?}",
        resp.callees
    );
}

#[test]
fn no_references_only_call_kind_returned() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert!(!resp.callers.is_empty());
    for c in &resp.callers {
        assert_eq!(
            c.kind,
            CallKind::Call,
            "when no REFERENCES exist, all callers must be Call kind"
        );
    }
}
