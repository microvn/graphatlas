//! v1.5 PR1a Phase F empirical test #2 — `wait_for_handle_release` + `is_busy_error`.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-empirical.md` S-002 AS-004.
//! Windows iterations (AS-005, AS-006) deferred to PR1b after Foundation PR3 adds
//! Windows CI matrix.
//!
//! This crate produces a CI artifact `target/lbug_lifecycle/close_rm_reopen.json`
//! consumed by PR9 (incremental pipeline) and PR6 (ga_reindex full rebuild).
//! Tests are `#[ignore]` by default; CI runs explicitly.
//!
//! C-4 (challenge): extracts `wait_for_handle_release` + `is_busy_error` into
//! `ga_index::lifecycle_helpers` as production module (NOT test-only), so PR6
//! reuses the same helper rather than reimplementing.

use ga_index::lifecycle_helpers::{is_busy_error, wait_for_handle_release};
use tempfile::TempDir;

#[test]
#[ignore = "Phase F gate — run via CI artifact pipeline"]
fn posix_wait_for_handle_release_is_immediate_noop() {
    // AS-004: POSIX has synchronous flock release. The helper exists only
    // for Windows handle-release lag; on POSIX it must short-circuit to
    // Ok(true) without probing the filesystem.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("nonexistent.db");

    // Even with a path that does not exist, POSIX must return Ok(true)
    // without I/O. (Windows arm in PR1b will probe and may error.)
    let result = wait_for_handle_release(&path);

    #[cfg(unix)]
    {
        let ok = result.as_ref().ok().copied();
        assert_eq!(
            ok,
            Some(true),
            "POSIX wait_for_handle_release must return Ok(true) immediately, got err={:?}",
            result.as_ref().err()
        );
    }

    // On Windows the test is non-trivial — covered by PR1b.
    #[cfg(windows)]
    let _ = result;
}

#[test]
fn is_busy_error_matches_busy_substring_case_insensitive() {
    // H-helper.2: matcher recognises lbug error messages containing
    // "busy" / "lock" / "already in use" regardless of case.
    let busy = lbug::Error::FailedQuery("Database is BUSY".to_string());
    let locked = lbug::Error::FailedQuery("file lock not acquired".to_string());
    let in_use = lbug::Error::FailedQuery("already in use by writer".to_string());
    let mixed = lbug::Error::FailedQuery("Resource Locked: try again".to_string());

    assert!(is_busy_error(&busy), "must match 'BUSY'");
    assert!(is_busy_error(&locked), "must match 'lock'");
    assert!(is_busy_error(&in_use), "must match 'already in use'");
    assert!(
        is_busy_error(&mixed),
        "must match 'Locked' case-insensitive"
    );
}

#[test]
fn is_busy_error_rejects_unrelated_errors() {
    // H-helper.3: non-contention errors must NOT trigger retry. Substring
    // overlap with unrelated words (e.g. "deadlock" → contains "lock") is
    // the trap; matcher must be conservative.
    let parse = lbug::Error::FailedQuery("Parser error: unexpected token".to_string());
    let schema = lbug::Error::FailedQuery("Table not found: NonexistentNode".to_string());

    assert!(
        !is_busy_error(&parse),
        "must not match generic parser error"
    );
    assert!(
        !is_busy_error(&schema),
        "must not match schema-missing error"
    );
}
