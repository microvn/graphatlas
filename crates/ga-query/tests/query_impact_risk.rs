//! Tools S-006 cluster C7 — integration smoke for runtime risk composite.
//! Unit tests for the pure maths live inside `impact/risk.rs`; these tests
//! verify the wiring from `impact()` → `Risk` field.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest, RiskLevel};
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
fn risk_is_low_when_symbol_unknown() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def alpha(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("nonexistent".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(resp.risk.score, 0.0);
    assert_eq!(resp.risk.level, RiskLevel::Low);
    assert!(resp.risk.reasons.is_empty());
}

#[test]
fn risk_score_increases_when_callers_exist_without_tests() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // target has a caller but no test file — test_gap > 0.
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("target".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        resp.risk.score > 0.0,
        "expected positive risk, got {:?}",
        resp.risk
    );
    assert!(
        resp.risk
            .reasons
            .iter()
            .any(|r| r.contains("test") || r.contains("blast") || r.contains("depth")),
        "reasons should cite a real dim: {:?}",
        resp.risk.reasons
    );
}

#[test]
fn risk_serializes_to_wire_shape() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def alpha(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let v = serde_json::to_value(&resp).unwrap();
    let risk = &v["risk"];
    assert!(risk["score"].is_number());
    assert!(matches!(
        risk["level"].as_str(),
        Some("low") | Some("medium") | Some("high")
    ));
    assert!(risk["reasons"].is_array());
}
