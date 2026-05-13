//! v1.4 S-001b — `ga_dead_code` OVERRIDES rescue.
//!
//! Spec: graphatlas-v1.4-data-model.md S-001b, AS-009 / AS-010 / AS-011.
//! Closes the FP class documented at CRG #363 + CMM #27 — methods that
//! override a parent method called via virtual dispatch should NOT be
//! flagged dead.
//!
//! Three rescue paths:
//! 1. Resolved OVERRIDES → parent has incoming CALLS (single-step):
//!    base case — child is targeted because parent is targeted.
//! 2. Multi-level chain (transitive): C overrides B overrides A; A has
//!    callers; both B and C must be rescued. Implementation: iterate to
//!    fixpoint over OVERRIDES edges in the targeted-set propagation.
//! 3. External / vendored parent (has_unresolved_override=true): parent
//!    couldn't resolve in-repo → child rescued via the flag (Tools-C12
//!    no synthetic edge; H1 fix from /mf-challenge bulk-apply).

use ga_index::Store;
use ga_query::dead_code::{dead_code, DeadCodeRequest};
use ga_query::indexer::build_index;
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn dead_pairs(resp: &ga_query::dead_code::DeadCodeResponse) -> Vec<(String, String)> {
    resp.dead
        .iter()
        .map(|e| (e.symbol.clone(), e.file.clone()))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────
// AS-009 — single-step rescue with curated fixture (no flag, anti-theatre
// via inverse fixture)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn override_method_with_called_parent_not_flagged_dead() {
    // Parent BaseHandler.handle is called externally (driver). 3 subclasses
    // override BaseHandler.handle but have NO direct callers. Without
    // OVERRIDES rescue they'd be flagged dead. With rescue, they shouldn't.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    // Parent class with handle(); has external caller.
    write(
        &repo.join("BaseHandler.java"),
        "package p;\npublic class BaseHandler { public void handle() {} }\n",
    );
    // 3 subclasses overriding handle(); no direct callers.
    for name in ["HandlerA", "HandlerB", "HandlerC"] {
        write(
            &repo.join(format!("{name}.java")),
            &format!("package p;\npublic class {name} extends BaseHandler {{ @Override public void handle() {{}} }}\n"),
        );
    }
    // Driver code that calls BaseHandler.handle (external caller of parent).
    write(
        &repo.join("Driver.java"),
        "package p;\npublic class Driver {\n  public void run(BaseHandler h) { h.handle(); }\n}\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let pairs = dead_pairs(&resp);

    for child in ["HandlerA.java", "HandlerB.java", "HandlerC.java"] {
        assert!(
            !pairs.contains(&("handle".to_string(), child.to_string())),
            "{child}::handle overrides BaseHandler.handle (called by Driver.run) and \
             must NOT be flagged dead by OVERRIDES rescue. dead_pairs={pairs:?}"
        );
    }
}

#[test]
fn non_override_method_with_zero_callers_still_flagged_dead() {
    // Inverse / anti-theatre fixture: a method NOT marked @Override and
    // with zero callers MUST still be flagged dead. Proves the rescue
    // logic only activates for actual override pairs, not arbitrary
    // 0-in-degree symbols.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("Orphan.java"),
        "package p;\npublic class Orphan { public void someMethod() {} }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let pairs = dead_pairs(&resp);

    assert!(
        pairs.contains(&("someMethod".to_string(), "Orphan.java".to_string())),
        "Orphan.someMethod has no callers and is not an override — MUST be flagged dead; \
         dead_pairs={pairs:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-010 — external/vendored parent rescue via has_unresolved_override flag
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn external_parent_override_rescued_via_has_unresolved_override() {
    // Subclass extends external/vendored class. Parent doesn't resolve in
    // repo → indexer sets has_unresolved_override=true → no OVERRIDES edge
    // (Tools-C12). dead_code rescue uses the flag as equivalent to a
    // resolved OVERRIDES edge for the rescue purpose.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("MyController.java"),
        "package my;\nimport com.vendor.framework.AbstractController;\n\
         public class MyController extends AbstractController { \
             @Override public void handleRequest() {} \
         }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let pairs = dead_pairs(&resp);

    assert!(
        !pairs.contains(&("handleRequest".to_string(), "MyController.java".to_string())),
        "MyController.handleRequest extends external AbstractController via @Override — \
         has_unresolved_override flag must rescue it from dead. dead_pairs={pairs:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-011 — 3-level inheritance chain transitive rescue
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn three_level_chain_all_levels_rescued_by_root_caller() {
    // A.foo() called externally; B extends A; B.foo() @Override has 0 direct
    // callers; C extends B; C.foo() @Override has 0 direct callers.
    // Both B.foo and C.foo must be rescued via transitive OVERRIDES walk.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("A.java"),
        "package p;\npublic class A { public void foo() {} }\n",
    );
    write(
        &repo.join("B.java"),
        "package p;\npublic class B extends A { @Override public void foo() {} }\n",
    );
    write(
        &repo.join("C.java"),
        "package p;\npublic class C extends B { @Override public void foo() {} }\n",
    );
    // Driver calls A.foo via virtual dispatch.
    write(
        &repo.join("Driver.java"),
        "package p;\npublic class Driver { public void run(A a) { a.foo(); } }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let pairs = dead_pairs(&resp);

    assert!(
        !pairs.contains(&("foo".to_string(), "B.java".to_string())),
        "B.foo overrides A.foo (called by Driver) → must be rescued. dead_pairs={pairs:?}"
    );
    assert!(
        !pairs.contains(&("foo".to_string(), "C.java".to_string())),
        "C.foo overrides B.foo (rescued via A.foo's caller) → transitive rescue MUST reach C. \
         dead_pairs={pairs:?}"
    );
}
