//! infra:S-002 — Phase B import-map disambiguation.
//!
//! AS-005: Python aliased import `from X import Foo as F; F()` resolves
//!         via import_map to `b.py::Foo`, NOT `__external__::F`.
//! AS-007: Three files define `Foo`; caller with `from a import Foo` hits
//!         the a.py Foo specifically (not arbitrary first-match).
//!
//! Note AS-006 (TS namespace `ns.foo()`) requires calls.rs changes to
//! preserve namespace prefix — deferred to KG-6 / Phase B.2 per
//! graphatlas-v1.1-infra.md Not in Scope.

use ga_index::Store;
use ga_query::{callees::callees, indexer::build_index};
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

fn callee_files_for(store: &Store, caller: &str) -> Vec<String> {
    let resp = callees(store, caller, None).expect("callees query");
    let mut out: Vec<String> = resp
        .callees
        .iter()
        .filter(|c| !c.external)
        .map(|c| c.file.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// AS-005 — `from mod.b import Foo as F; F()` resolves to b.py::Foo
/// (NOT __external__::F, NOT ambiguous fallback).
#[test]
fn aliased_import_resolves_to_original_definition() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("mod/b.py"), "def Foo():\n    pass\n");
    write(
        &repo.join("a.py"),
        "from mod.b import Foo as F\n\ndef caller():\n    F()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let files = callee_files_for(&store, "caller");
    assert!(
        files.iter().any(|f| f.ends_with("mod/b.py")),
        "AS-005: aliased-import F should resolve to mod/b.py::Foo; got {:?}",
        files
    );
    assert!(
        !files.iter().any(|f| f == "__external__"),
        "AS-005: alias must NOT fall through to __external__::F; got {:?}",
        files
    );
}

/// AS-007 — Three `Foo` defs across files; caller with `from a import Foo`
/// hits a.py::Foo specifically (regression guard for existing Phase B
/// scaffold — this should already work with unqualified import_map).
#[test]
fn same_name_ambiguity_resolved_via_import_hint() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def Foo():\n    pass  # the one we want\n",
    );
    write(&repo.join("b.py"), "def Foo():\n    pass  # red herring\n");
    write(&repo.join("c.py"), "def Foo():\n    pass  # red herring\n");
    write(
        &repo.join("x.py"),
        "from a import Foo\n\ndef caller():\n    Foo()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let files = callee_files_for(&store, "caller");
    assert_eq!(
        files,
        vec!["a.py".to_string()],
        "AS-007: `from a import Foo; Foo()` must resolve to a.py::Foo only; got {:?}",
        files
    );
}
