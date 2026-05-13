//! Tools S-006 cluster C0 — scaffold coverage.
//!
//! Pins the response shape + stub behavior. Real AS-012..016 behavior tests
//! land in clusters C1..C10.

use ga_index::Store;
use ga_query::{impact, ImpactRequest, ImpactResponse, RiskLevel};
use std::fs;
use tempfile::TempDir;

fn empty_store() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    (tmp, store)
}

#[test]
fn impact_stub_returns_empty_response_for_symbol_input() {
    let (_tmp, store) = empty_store();
    let req = ImpactRequest {
        symbol: Some("some_fn".into()),
        ..Default::default()
    };
    let resp = impact(&store, &req).expect("stub must not error");
    assert!(resp.impacted_files.is_empty());
    assert!(resp.affected_tests.is_empty());
    assert!(resp.affected_routes.is_empty());
    assert!(resp.affected_configs.is_empty());
    assert!(resp.break_points.is_empty());
    assert_eq!(resp.risk.score, 0.0);
    assert_eq!(resp.risk.level, RiskLevel::Low);
}

#[test]
fn impact_request_deserializes_symbol_shape() {
    let raw = serde_json::json!({ "symbol": "foo", "file": "bar.py" });
    let req: ImpactRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(req.symbol.as_deref(), Some("foo"));
    assert_eq!(req.file.as_deref(), Some("bar.py"));
    assert!(req.changed_files.is_none());
    assert!(req.diff.is_none());
}

#[test]
fn impact_request_deserializes_changed_files_shape() {
    let raw = serde_json::json!({ "changed_files": ["a.py", "b.py"] });
    let req: ImpactRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(
        req.changed_files.as_deref(),
        Some(&["a.py".to_string(), "b.py".to_string()][..])
    );
}

#[test]
fn impact_request_deserializes_diff_shape() {
    let raw = serde_json::json!({ "diff": "--- a/x\n+++ b/x\n" });
    let req: ImpactRequest = serde_json::from_value(raw).unwrap();
    assert!(req.diff.is_some());
}

#[test]
fn impact_request_deserializes_max_depth() {
    let raw = serde_json::json!({ "symbol": "x", "max_depth": 2 });
    let req: ImpactRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(req.max_depth, Some(2));
}

#[test]
fn impact_response_serializes_roundtrip() {
    let resp = ImpactResponse::default();
    let v = serde_json::to_value(&resp).unwrap();
    // Wire contract — every top-level key present.
    for key in [
        "impacted_files",
        "affected_tests",
        "affected_routes",
        "affected_configs",
        "risk",
        "break_points",
        "meta",
    ] {
        assert!(v.get(key).is_some(), "missing key {key} in {v}");
    }
    assert_eq!(v["risk"]["level"], "low");
}
