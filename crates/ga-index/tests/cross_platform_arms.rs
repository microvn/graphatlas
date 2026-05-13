//! v1.5 PR3 foundation S-002 — cross-platform arm regression tests.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-foundation.md`
//! S-002 AS-006..009.
//!
//! These tests verify that the `cfg(target_os = "...")` arms compile + behave
//! sensibly on each platform. Empirical Windows runtime verification still
//! requires the Windows CI runner (added in same PR via .github/workflows/ci.yml).

#[cfg(unix)]
#[test]
fn unix_chmod_0600_returns_ok_on_existing_file() {
    // AS-007 happy path (Unix arm). The cfg(unix) implementation of
    // chmod_0600 must succeed on a regular file.
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("file");
    std::fs::write(&path, b"x").unwrap();
    ga_index::cache::chmod_0600(&path).expect("chmod_0600 unix happy path");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(windows)]
#[test]
fn windows_chmod_0600_is_noop_no_panic() {
    // AS-007 Windows arm. The cfg(not(unix)) implementation must compile
    // AND not panic — it is a documented no-op (Windows ACLs are out of
    // scope for v1.5; tracked in Foundation-C8 Windows row).
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("file");
    std::fs::write(&path, b"x").unwrap();
    // The function must return Ok without touching ACLs.
    ga_index::cache::chmod_0600(&path).expect("chmod_0600 windows no-op");
    // No assertion on file metadata — ACL handling deferred.
}

#[test]
fn chmod_0600_compiles_and_runs_on_current_platform() {
    // AS-007 smoke: regardless of platform, `chmod_0600` must be callable
    // and return Ok for a regular file. This is the compile-gate for the
    // cfg-arm parity (both unix and not(unix) impls exist).
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("smoke");
    std::fs::write(&path, b"x").unwrap();
    let result = ga_index::cache::chmod_0600(&path);
    assert!(result.is_ok(), "chmod_0600 must be Ok on current platform");
}
