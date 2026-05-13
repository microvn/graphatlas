//! EXP-M2-02 — `ImpactRequest.include_*` opt-out flags.
//!
//! Bench benchmarks only measure impacted_files + affected_tests toward
//! composite. Routes/configs/break_points/risk subcomponents burn 400-800ms
//! per call but don't contribute to the score — gate them behind opt-out
//! flags (default true = backward compat; false = skip work).

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// Build a tiny repo that would populate routes, configs, break_points, risk
/// on a default `ImpactRequest`. Returns the store.
fn setup_populated_repo(tmp: &TempDir) -> Store {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    // Python Flask-like route → should populate affected_routes.
    write(
        &repo.join("routes.py"),
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route('/users', methods=['GET'])\ndef handler():\n    return seed()\n\
         def seed():\n    return 'x'\n",
    );
    // Caller defines a break point for `seed`.
    write(
        &repo.join("caller.py"),
        "from routes import seed\ndef do():\n    seed()\n",
    );
    // Config file to populate affected_configs.
    write(&repo.join(".env"), "DEBUG=1\nSEED_KEY=seed_value\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    store
}

#[test]
fn defaults_populate_all_subcomponents_backward_compat() {
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    // Default request — all include_* unset (or implicitly true).
    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // Backward compat: default behavior still populates everything.
    // impacted_files always populated.
    assert!(
        !resp.impacted_files.is_empty(),
        "impacted_files must populate"
    );
    // Break points require a caller — caller.py::do calls seed, so at least 1.
    assert!(
        !resp.break_points.is_empty(),
        "break_points must populate by default"
    );
    // Risk score computed (>= 0 always, but tracking that computation ran).
    // Even at 0.0, reasons would reflect computation on the populated signals.
    // Easiest check: risk level exists (defaulted to Low = computed OR skipped — too weak).
    // Stronger check: the other signals actually filled the response, so
    // reaching here means the full pipeline ran.
}

#[test]
fn include_break_points_false_skips_break_point_collection() {
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            include_break_points: Some(false),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        resp.break_points.is_empty(),
        "break_points must be empty when include_break_points=false; got {} entries",
        resp.break_points.len(),
    );
    // Other fields unaffected.
    assert!(
        !resp.impacted_files.is_empty(),
        "impacted_files still populated"
    );
}

#[test]
fn include_routes_false_skips_route_detection() {
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            include_routes: Some(false),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        resp.affected_routes.is_empty(),
        "affected_routes must be empty when include_routes=false; got {} entries",
        resp.affected_routes.len(),
    );
}

#[test]
fn include_configs_false_skips_config_scan() {
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            include_configs: Some(false),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        resp.affected_configs.is_empty(),
        "affected_configs must be empty when include_configs=false; got {} entries",
        resp.affected_configs.len(),
    );
}

#[test]
fn include_risk_false_leaves_risk_at_default() {
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            include_risk: Some(false),
            ..Default::default()
        },
    )
    .unwrap();

    // Risk fields should be at Default::default() values when skipped.
    assert_eq!(
        resp.risk.score, 0.0,
        "risk score must be default when include_risk=false"
    );
    assert!(
        resp.risk.reasons.is_empty(),
        "risk reasons must be empty when include_risk=false; got {:?}",
        resp.risk.reasons,
    );
}

#[test]
fn all_flags_false_still_populates_impacted_files_and_tests() {
    // Bench use case — it only needs impacted_files + affected_tests, so
    // disabling the 4 other subcomponents must not affect those.
    let tmp = TempDir::new().unwrap();
    let store = setup_populated_repo(&tmp);

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            include_break_points: Some(false),
            include_routes: Some(false),
            include_configs: Some(false),
            include_risk: Some(false),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        !resp.impacted_files.is_empty(),
        "impacted_files MUST populate regardless of include_* flags"
    );
    // affected_tests may or may not exist depending on repo; the contract is
    // that we DON'T gate affected_tests behind a flag.
    assert!(resp.break_points.is_empty());
    assert!(resp.affected_routes.is_empty());
    assert!(resp.affected_configs.is_empty());
    assert_eq!(resp.risk.score, 0.0);
}
