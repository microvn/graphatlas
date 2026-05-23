//! CORE-3 regression suite (2026-05-22) — `ga_impact` must not flag a
//! same-named symbol from a *different* language as `calls_seed_directly` /
//! `called_by_seed_directly` with confidence 1.0.
//!
//! Real-world evidence: cardshield round 2 — TS `checkLicenseExpiry` impact
//! pulled in `plugin/rotor-paypal/tests/support/ObjectCacheStub.php` as a
//! depth-1 callee with confidence 1.0. PHP file is unrelated runtime code;
//! the cross-language match is a name collision, not a true call edge.
//!
//! Fix shape: when seed file's language differs from impacted file's
//! language, downgrade the relation to `shares_function_name` + lower
//! confidence (≤ 0.5).
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! CORE-3.

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

#[test]
fn cross_language_homonym_def_classified_correctly() {
    // CORE-3 part 1 — when a PHP file defines a function with the same name
    // as a TS seed, the PHP file appears as a depth-0 homonym. It must NOT
    // be classified as `changed_directly` (would falsely claim PHP is the
    // change target). `shares_function_name` is the correct outcome.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("app.ts"),
        "export function do_thing(): boolean {\n  return true;\n}\n",
    );
    write(
        &repo.join("legacy.php"),
        "<?php\nfunction do_thing(): bool {\n    return false;\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("do_thing".into()),
            file: Some("app.ts".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let php = resp.impacted_files.iter().find(|f| f.path == "legacy.php");
    if let Some(f) = php {
        assert!(
            f.relation_to_seed == "shares_function_name",
            "PHP homonym must be shares_function_name, got {:?}",
            f.relation_to_seed
        );
        assert!(f.confidence < 1.0, "must be < 1.0: {f:?}");
    }
}

#[test]
fn cross_language_caller_edge_downgraded() {
    // Regression: CORE-3 — TS seed had a PHP file flagged as
    // `calls_seed_directly` / `called_by_seed_directly` at confidence 1.0
    // because of a cross-file CALL edge produced by name-based fallback
    // resolution (PHP function with same name as a TS callee).
    //
    // Setup: TS file `app.ts` defines `seed_fn`. TS file `caller.ts` calls
    // a function named `helper` (resolved across files by name). PHP file
    // `legacy.php` also defines `helper`. The indexer may emit CALL edges
    // from caller.ts → both `helper` defs (TS + PHP). The PHP edge must
    // surface as cross-lang collision, NOT as direct call at conf 1.0.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.ts"),
        "export function seed_fn(): boolean {\n  helper();\n  return true;\n}\n\n\
         export function helper(): void {}\n",
    );
    write(
        &repo.join("legacy.php"),
        "<?php\nfunction helper(): void {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed_fn".into()),
            file: Some("a.ts".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // If legacy.php surfaces (cross-lang CALL edge to `helper`), it must be
    // downgraded — never `calls_seed_directly` at conf 1.0.
    for f in resp
        .impacted_files
        .iter()
        .filter(|f| f.path == "legacy.php")
    {
        let relation = &f.relation_to_seed;
        let bad = relation == "calls_seed_directly" || relation == "called_by_seed_directly";
        assert!(
            !(bad && (f.confidence - 1.0).abs() < 1e-6),
            "cross-lang PHP edge must be downgraded, got {} conf {} on {:?}",
            relation,
            f.confidence,
            f.path
        );
    }
}

#[test]
fn cross_language_method_call_name_collision_downgraded() {
    // Regression: CORE-3 (confirmed on cardshield 2026-05-22) — TS function
    // `checkLicenseExpiry` calls `Date.now()`. PHP class `TestClock` defines
    // `now()`. Indexer matches CALLS edges by method name only → PHP file
    // surfaces as `calls_seed_directly` for the TS seed at conf 1.0.
    //
    // Repro: TS seed calls a method (any builtin-style call); PHP file has
    // a class method with the same name. Without language gating, the
    // indexer's name-only join creates the false cross-lang edge.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("app.ts"),
        "export function seed(value: string): number {\n\
        \x20\x20const d = new Date();\n\
        \x20\x20return d.now();\n\
        }\n",
    );
    write(
        &repo.join("clock.php"),
        "<?php\nfinal class TestClock {\n    public static function now(): int { return 0; }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            file: Some("app.ts".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // PHP file must not appear with `calls_seed_directly` conf 1.0. Either
    // not in list at all (preferred) or downgraded to `shares_function_name`
    // / similar with conf < 1.0.
    for f in resp
        .impacted_files
        .iter()
        .filter(|f| f.path.ends_with(".php"))
    {
        let bad = f.relation_to_seed == "calls_seed_directly" && (f.confidence - 1.0).abs() < 1e-6;
        assert!(
            !bad,
            "PHP cross-lang edge must NOT be conf-1.0 calls_seed_directly: {f:?}"
        );
    }
}

#[test]
fn same_language_caller_keeps_direct_classification() {
    // Sanity check: within-language relations stay at conf 1.0 with the
    // direct-call classification.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.ts"),
        "export function helper(): boolean {\n  return true;\n}\n",
    );
    write(
        &repo.join("b.ts"),
        "import {helper} from './a';\nexport function driver(): void {\n  helper();\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("helper".into()),
            file: Some("a.ts".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let b = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "b.ts")
        .expect("b.ts should be impacted");
    assert!(
        (b.confidence - 1.0).abs() < 1e-6,
        "same-lang caller stays conf 1.0: {:?}",
        b
    );
}
