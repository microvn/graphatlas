//! Bench S-001 AS-002 + AS-003 — error paths before any retriever runs.

use ga_bench::{runner, BenchError};
use std::fs;
use tempfile::TempDir;

#[test]
fn missing_fixture_dir_reports_as002() {
    // AS-002: `git submodule update --init --recursive` hint.
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("benches/fixtures/ghost");
    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{"schema_version":1,"uc":"callers","fixture":"ghost","tasks":[]}"#,
    )
    .unwrap();

    let err = runner::validate_inputs(&missing, &gt_path).unwrap_err();
    match err {
        BenchError::FixtureMissing { path } => {
            assert!(path.contains("ghost"), "path hint: {path}");
        }
        other => panic!("expected FixtureMissing, got {other:?}"),
    }
}

#[test]
fn empty_fixture_dir_reports_as002() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches/fixtures/empty");
    fs::create_dir_all(&fixture).unwrap();
    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{"schema_version":1,"uc":"callers","fixture":"empty","tasks":[]}"#,
    )
    .unwrap();

    let err = runner::validate_inputs(&fixture, &gt_path).unwrap_err();
    assert!(matches!(err, BenchError::FixtureMissing { .. }));
}

#[test]
fn gt_schema_mismatch_reports_as003() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches/fixtures/mini");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("a.py"), "def a(): pass\n").unwrap();
    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{"schema_version":999,"uc":"callers","fixture":"mini","tasks":[]}"#,
    )
    .unwrap();

    let err = runner::validate_inputs(&fixture, &gt_path).unwrap_err();
    match err {
        BenchError::SchemaMismatch { got, expected } => {
            assert_eq!(got, 999);
            assert_eq!(expected, 1);
        }
        other => panic!("expected SchemaMismatch, got {other:?}"),
    }
}

#[test]
fn gt_malformed_json_reports_error() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches/fixtures/mini");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("a.py"), "def a(): pass\n").unwrap();
    let gt_path = tmp.path().join("gt.json");
    fs::write(&gt_path, "{ this is not json").unwrap();

    let err = runner::validate_inputs(&fixture, &gt_path).unwrap_err();
    assert!(matches!(err, BenchError::GroundTruthMalformed { .. }));
}

#[test]
fn validate_passes_when_fixture_and_gt_ok() {
    let tmp = TempDir::new().unwrap();
    let fixture = tmp.path().join("benches/fixtures/mini");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("a.py"), "def a(): pass\n").unwrap();
    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{"schema_version":1,"uc":"callers","fixture":"mini","tasks":[]}"#,
    )
    .unwrap();

    let gt = runner::validate_inputs(&fixture, &gt_path).unwrap();
    assert_eq!(gt.uc, "callers");
    assert_eq!(gt.schema_version, 1);
}
