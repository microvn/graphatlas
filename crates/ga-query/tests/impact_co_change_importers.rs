//! EXP-M2-11 — co-change + importer intersection pool in impact().
//!
//! Mirrors `extract-seeds.ts:491-538` Phase C algorithm: (files that import
//! seed module at HEAD) ∩ (files co-changed with seed ≥3 times last 100
//! commits). H-M6 harness (`bench-results/m2-h6-stability-audit.md`) confirmed
//! this variant passes S/N ≥ 1:5 gate (2.04 observed) with +18.9%
//! blast_radius lift.
//!
//! Tests here exercise:
//! 1. `signals::co_change_importers::compute_co_change_importers` — new fn
//! 2. `ImpactRequest.include_co_change_importers` flag (default true)
//! 3. `ImpactReason::CoChange` variant on returned `ImpactedFile`
//!
//! All tests `#[ignore]` because they need the axum submodule fixture +
//! real git history. Run: `cargo test -p ga-query --release
//! impact_co_change_importers -- --ignored --nocapture`.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::signals::co_change_importers::compute_co_change_importers;
use ga_query::{impact, ImpactReason, ImpactRequest};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// tempfile::TempDir creates dirs with 0755 on macOS — GRAPHATLAS_CACHE_DIR
/// rejects anything > 0700. Tighten before opening Store.
fn tight_tempdir() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    tmp
}

fn axum_fixture() -> Option<PathBuf> {
    let root = workspace_root();
    let p = root.join("benches/fixtures/axum");
    if p.join(".git").exists() || p.join("Cargo.toml").exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
#[ignore]
fn compute_returns_nonempty_on_axum_cookie_mod() {
    let Some(fixture) = axum_fixture() else {
        eprintln!("[SKIP] axum fixture missing");
        return;
    };
    // axum-extra cookie/mod.rs has sibling files private.rs + signed.rs
    // that co-change heavily (per axum-34d1fbc0 task's should_touch_files).
    // Manual sanity check on fixture confirms ≥2 hits at threshold ≥3.
    let result =
        compute_co_change_importers(&fixture, "axum-extra/src/extract/cookie/mod.rs", "rust", 3);
    assert!(
        !result.is_empty(),
        "expected ≥1 co-change importer hit on axum-extra cookie/mod, got empty"
    );
    assert!(
        !result.contains("axum-extra/src/extract/cookie/mod.rs"),
        "seed file must not appear in its own co-change importers"
    );
    // B' intersection should be tight (H-M6 harness: axum ~5-10 hits per task).
    assert!(
        result.len() < 50,
        "intersection should be tight; got {} files",
        result.len()
    );
}

#[test]
#[ignore]
fn compute_returns_empty_for_unsupported_lang() {
    let Some(fixture) = axum_fixture() else {
        return;
    };
    let result = compute_co_change_importers(
        &fixture,
        "axum/src/handler/mod.rs",
        "cobol", // unsupported
        3,
    );
    assert!(result.is_empty(), "unsupported lang must return empty set");
}

#[test]
#[ignore]
fn impact_with_flag_on_adds_co_change_files() {
    let Some(fixture) = axum_fixture() else {
        eprintln!("[SKIP] axum fixture missing");
        return;
    };
    let tmp = tight_tempdir();
    let store = Store::open_with_root(tmp.path(), &fixture).unwrap();
    build_index(&store, &fixture).unwrap();

    let req_off = ImpactRequest {
        symbol: Some("CookieJar".into()),
        include_break_points: Some(false),
        include_routes: Some(false),
        include_configs: Some(false),
        include_risk: Some(false),
        include_co_change_importers: Some(false),
        ..Default::default()
    };
    let req_on = ImpactRequest {
        include_co_change_importers: Some(true),
        ..req_off.clone()
    };

    let resp_off = impact(&store, &req_off).unwrap();
    let resp_on = impact(&store, &req_on).unwrap();

    assert!(
        resp_on.impacted_files.len() >= resp_off.impacted_files.len(),
        "flag=on must not shrink output; on={} off={}",
        resp_on.impacted_files.len(),
        resp_off.impacted_files.len()
    );

    let co_change_count = resp_on
        .impacted_files
        .iter()
        .filter(|f| matches!(f.reason, ImpactReason::CoChange))
        .count();
    assert!(
        co_change_count >= 1,
        "expected ≥1 ImpactReason::CoChange entry when flag=on, got 0"
    );
    // Flag off must never produce CoChange entries.
    let off_co_change = resp_off
        .impacted_files
        .iter()
        .filter(|f| matches!(f.reason, ImpactReason::CoChange))
        .count();
    assert_eq!(off_co_change, 0, "flag=off must not emit CoChange reason");
}

#[test]
#[ignore]
fn impact_co_change_default_behavior_is_off() {
    // EXP-M2-11 — default is `false` because git-subprocess fan-out blows
    // p95 latency 4× (422→1867ms). Opt-in only for callers that tolerate
    // the cost in exchange for +38% blast_radius coverage.
    let Some(fixture) = axum_fixture() else {
        return;
    };
    let tmp = tight_tempdir();
    let store = Store::open_with_root(tmp.path(), &fixture).unwrap();
    build_index(&store, &fixture).unwrap();

    let req_default = ImpactRequest {
        symbol: Some("CookieJar".into()),
        include_break_points: Some(false),
        include_routes: Some(false),
        include_configs: Some(false),
        include_risk: Some(false),
        ..Default::default()
    };
    let req_off = ImpactRequest {
        include_co_change_importers: Some(false),
        ..req_default.clone()
    };
    let resp_default = impact(&store, &req_default).unwrap();
    let resp_off = impact(&store, &req_off).unwrap();
    assert_eq!(
        resp_default.impacted_files.len(),
        resp_off.impacted_files.len(),
        "default flag must match Some(false) behavior (opt-in only)"
    );
    // No CoChange entries in default path.
    let default_co_change = resp_default
        .impacted_files
        .iter()
        .filter(|f| matches!(f.reason, ImpactReason::CoChange))
        .count();
    assert_eq!(
        default_co_change, 0,
        "default path must not emit CoChange reason (opt-in only)"
    );
}
