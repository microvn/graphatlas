//! CORE-2 regression suite (2026-05-22) — ambiguity-first multi-def resolution
//! for `callers` / `callees` / `impact`.
//!
//! When a symbol has >1 definition in the indexed graph AND the caller did not
//! pass a `file:` narrowing hint, the response must NOT fan out a mix of
//! same-name defs' edges at confidence 0.6. Instead, return a structured
//! `disambiguation` payload listing the candidate defs so the LLM (or human)
//! can retry with a hint or qualified name.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! — tokio `block_on` (5 defs) produced 17k token / 445 entries; gin `Default`
//! (3 defs) produced 30+ noise entries.
//!
//! Legacy behaviour is preserved behind `GA_AMBIGUITY_LEGACY=1` for any
//! downstream that depends on the old fan-out.

use ga_index::Store;
use ga_query::{callees, callers, impact, indexer::build_index, ImpactRequest};
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

/// Build a repo with TWO defs of `Default` — mimics gin `Default`
/// (gin/Default vs binding/Default). Each def has its own caller.
fn build_multi_def_repo(repo: &Path) {
    write(
        &repo.join("a.py"),
        "def Default():\n    return 'a'\n\ndef caller_a():\n    Default()\n",
    );
    write(
        &repo.join("b.py"),
        "def Default():\n    return 'b'\n\ndef caller_b():\n    Default()\n",
    );
}

#[test]
fn callers_multi_def_no_hint_returns_ambiguous() {
    // Regression: CORE-2 — without a hint, GA used to fan out callers of all
    // same-name defs at confidence 0.6 (tokio `block_on`: 445 entries / 17k tok).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    build_multi_def_repo(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "Default", None).unwrap();

    let dis = resp
        .disambiguation
        .as_ref()
        .expect("multi-def + no hint must populate disambiguation");
    assert_eq!(dis.reason, "ambiguous", "reason field");
    assert!(
        dis.candidates.len() >= 2,
        "at least 2 candidates: {:?}",
        dis.candidates
    );
    let files: Vec<&str> = dis.candidates.iter().map(|c| c.file.as_str()).collect();
    assert!(files.contains(&"a.py"), "{:?}", files);
    assert!(files.contains(&"b.py"), "{:?}", files);
    assert!(
        resp.callers.is_empty(),
        "callers must be empty when ambiguous: {:?}",
        resp.callers
    );
    assert!(resp.meta.symbol_found);
}

#[test]
fn callers_single_def_unchanged_no_disambiguation() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def lonely():\n    pass\n\ndef caller():\n    lonely()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "lonely", None).unwrap();
    assert!(
        resp.disambiguation.is_none(),
        "single-def must not gate: {:?}",
        resp.disambiguation
    );
    assert_eq!(resp.callers.len(), 1);
    assert_eq!(resp.callers[0].confidence, 1.0);
}

#[test]
fn callers_multi_def_with_hint_unchanged_behaviour() {
    // Pre-CORE-2 behaviour: file hint narrows the exact def to conf 1.0 and
    // surfaces other-file polymorphic callers at conf 0.6. Must continue to
    // pass — the ambiguity gate only fires when NO hint is given.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    build_multi_def_repo(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "Default", Some("a.py")).unwrap();
    assert!(
        resp.disambiguation.is_none(),
        "file hint resolves ambiguity, no disambiguation expected: {:?}",
        resp.disambiguation
    );
    let exact: Vec<_> = resp
        .callers
        .iter()
        .filter(|c| (c.confidence - 1.0).abs() < 1e-6)
        .collect();
    assert!(!exact.is_empty(), "at least one conf=1.0 caller");
    assert!(exact.iter().any(|c| c.symbol == "caller_a"));
}

#[test]
fn callees_multi_def_no_hint_returns_ambiguous() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def helper(): pass\ndef Default():\n    helper()\n",
    );
    write(
        &repo.join("b.py"),
        "def util(): pass\ndef Default():\n    util()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "Default", None).unwrap();
    let dis = resp
        .disambiguation
        .as_ref()
        .expect("multi-def callees must gate ambiguity");
    assert!(dis.candidates.len() >= 2);
    assert!(resp.callees.is_empty(), "callees: {:?}", resp.callees);
}

#[test]
fn callees_single_def_unchanged() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def helper():\n    pass\n\ndef caller():\n    helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callees(&store, "caller", None).unwrap();
    assert!(resp.disambiguation.is_none());
    assert!(resp.callees.iter().any(|c| c.symbol == "helper"));
}

#[test]
fn impact_multi_def_no_hint_returns_ambiguous() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    build_multi_def_repo(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("Default".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let dis = resp
        .disambiguation
        .as_ref()
        .expect("multi-def impact must gate ambiguity");
    assert!(dis.candidates.len() >= 2);
    assert!(
        resp.impacted_files.is_empty(),
        "no traversal when ambiguous: {:?}",
        resp.impacted_files
    );
    assert!(resp.break_points.is_empty());
    assert!(resp.affected_tests.is_empty());
    assert!(resp.affected_configs.is_empty());
}

#[test]
fn impact_with_file_hint_resolves_ambiguity() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    build_multi_def_repo(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("Default".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        resp.disambiguation.is_none(),
        "file hint resolves: {:?}",
        resp.disambiguation
    );
    assert!(!resp.impacted_files.is_empty());
}

// Note: GA_AMBIGUITY_LEGACY=1 opt-out is verified manually — automated test
// would require mutating env vars, which workspace `unsafe_code = forbid`
// disallows. Verified via shell:
//   $ GA_AMBIGUITY_LEGACY=1 cargo run -p graphatlas -- mcp
// then issue ga_callers Default — old fan-out at confidence 0.6 returns.
//
// If a future change moves the opt-out from env to a process-local flag,
// re-add this test:
//
// #[test]
// fn legacy_flag_bypasses_ambiguity_gate() {
//     let tmp = TempDir::new().unwrap();
//     let (cache, repo) = setup(&tmp);
//     build_multi_def_repo(&repo);
//     let store = Store::open_with_root(&cache, &repo).unwrap();
//     build_index(&store, &repo).unwrap();
//     ga_query::set_ambiguity_legacy_for_tests(true);
//     let resp = callers(&store, "Default", None).unwrap();
//     ga_query::set_ambiguity_legacy_for_tests(false);
//     assert!(resp.disambiguation.is_none());
//     assert!(resp.callers.iter().all(|c| (c.confidence - 0.6).abs() < 1e-6));
// }

#[test]
fn disambiguation_candidate_shape_matches_contract() {
    // Candidates must carry qualified_name + file + line + kind so the LLM
    // can re-issue the call with `file:` hint or a qualified target.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    build_multi_def_repo(&repo);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "Default", None).unwrap();
    let dis = resp.disambiguation.expect("ambiguous");
    for c in &dis.candidates {
        assert!(!c.qualified_name.is_empty(), "qualified_name: {c:?}");
        assert!(
            c.qualified_name.contains("::"),
            "qualified_name format `<file>::<symbol>`: {c:?}"
        );
        assert!(!c.file.is_empty(), "file: {c:?}");
        assert!(c.line > 0, "line: {c:?}");
        assert!(
            !c.kind.is_empty(),
            "kind (function/method/class/...): {c:?}"
        );
    }
}
