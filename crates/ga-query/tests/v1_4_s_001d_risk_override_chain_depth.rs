//! v1.4 S-001d — `ga_risk` `override_chain_depth` factor (wiring only).
//!
//! Spec: graphatlas-v1.4-data-model.md S-001d, AS-014. Wiring assertion
//! only — factor is present and populated correctly per the OVERRIDES
//! traversal. Score formula behavior NOT asserted in v1.4 (deferred to
//! a future EXP-RISK-OVERRIDE-WEIGHT entry under EXPERIMENTS.md gated on
//! real fixture leaderboard data).

use ga_index::Store;
use ga_query::blame::BlameMiner;
use ga_query::indexer::build_index;
use ga_query::risk::{risk, RiskRequest};
use std::fs;
use tempfile::TempDir;

struct StubMiner;
impl BlameMiner for StubMiner {
    fn commit_subjects_since(&self, _file: &str, _days: u32) -> Vec<String> {
        Vec::new()
    }
}

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn risk_response_includes_override_chain_depth_field() {
    // Wiring test: response includes the new factor. Method is NOT an
    // override → depth=0 (or None handled gracefully).
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("Animal.java"),
        "package p;\npublic class Animal { public void speak() {} }\n",
    );
    write(
        &repo.join("Driver.java"),
        "package p;\npublic class Driver { public void run(Animal a) { a.speak(); } }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RiskRequest {
        symbol: Some("speak".to_string()),
        file_hint: Some("Animal.java".to_string()),
        ..Default::default()
    };
    let resp = risk(&store, &StubMiner, &req).expect("risk ok");

    // Animal.speak has no parent — depth = 0 OR None.
    let depth = resp.override_chain_depth.unwrap_or(0);
    assert_eq!(
        depth, 0,
        "non-override method must report depth=0; got Some({:?})",
        resp.override_chain_depth
    );
}

#[test]
fn override_chain_depth_monotonic_with_inheritance_depth() {
    // 3-level chain: A.foo (depth 0), B extends A (B.foo depth 1),
    // C extends B (C.foo depth 2). Verifies the OVERRIDES* traversal
    // walks the chain to the root.
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
    write(
        &repo.join("Driver.java"),
        "package p;\npublic class Driver { public void run(A a) { a.foo(); } }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // A.foo — depth 0 (root, no override)
    let req_a = RiskRequest {
        symbol: Some("foo".to_string()),
        file_hint: Some("A.java".to_string()),
        ..Default::default()
    };
    let resp_a = risk(&store, &StubMiner, &req_a).expect("risk A.foo ok");
    let depth_a = resp_a.override_chain_depth.unwrap_or(0);

    // B.foo — depth 1 (overrides A.foo)
    let req_b = RiskRequest {
        symbol: Some("foo".to_string()),
        file_hint: Some("B.java".to_string()),
        ..Default::default()
    };
    let resp_b = risk(&store, &StubMiner, &req_b).expect("risk B.foo ok");
    let depth_b = resp_b.override_chain_depth.unwrap_or(0);

    // C.foo — depth 2 (overrides B.foo which overrides A.foo)
    let req_c = RiskRequest {
        symbol: Some("foo".to_string()),
        file_hint: Some("C.java".to_string()),
        ..Default::default()
    };
    let resp_c = risk(&store, &StubMiner, &req_c).expect("risk C.foo ok");
    let depth_c = resp_c.override_chain_depth.unwrap_or(0);

    assert_eq!(depth_a, 0, "A.foo (root) must have depth=0; got {depth_a}");
    assert_eq!(depth_b, 1, "B.foo overrides A.foo → depth=1; got {depth_b}");
    assert_eq!(depth_c, 2, "C.foo overrides B.foo → depth=2; got {depth_c}");
}
