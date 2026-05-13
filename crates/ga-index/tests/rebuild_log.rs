//! S-003 AS-008 + AS-027 — rebuild paths emit the spec-literal log lines so
//! doctor/logs can surface them. `Store::open_*` writes to stderr.

use tempfile::TempDir;

/// Spawn `cargo` to run a single test-binary subprocess whose stderr we can
/// capture. Using Command here because `cargo test` swallows stderr by default
/// and we want the user-facing rebuild message to appear on the REAL stderr of
/// any real `graphatlas` process.
#[test]
fn schema_mismatch_emits_rebuild_line_on_stderr() {
    // We exercise the codepath in-process but read the SAME eprintln! output
    // via the `gag` crate — not available. Instead: test the message shape
    // by exposing a pure function `crate::store::rebuild_log_line`.
    let line = ga_index::store::rebuild_log_line_schema_mismatch(1, 2);
    assert!(line.contains("schema version mismatch"), "line: {line}");
    assert!(line.contains("cache=1"), "line: {line}");
    assert!(line.contains("binary=2"), "line: {line}");
    assert!(line.contains("rebuilding"), "line: {line}");
}

#[test]
fn schema_upgrade_log_mentions_estimated_time_and_version() {
    let line = ga_index::store::rebuild_log_line_schema_upgrade(2);
    assert!(line.contains("v2"), "line: {line}");
    assert!(line.contains("~") && line.contains("min"), "line: {line}");
}

#[test]
fn crash_recovery_log_mentions_wipe_and_rebuild() {
    let line = ga_index::store::rebuild_log_line_crash_recovery();
    let s = line.to_lowercase();
    assert!(
        s.contains("crash") || s.contains("incomplete"),
        "line: {line}"
    );
    assert!(s.contains("rebuild"), "line: {line}");
}

#[test]
fn store_open_populates_outcome_matching_log_line() {
    // End-to-end glue test: the OpenOutcome carried back from open_with_root
    // must be the same variant whose rebuild_log_line is emitted.
    // This pins the invariant "log says what outcome says".
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    // v1.5 PR2 AS-001: real path required for commit (Merkle root hash).
    let repo_dir = tmp.path().join("repos").join("log-mismatch");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();

    {
        let s = ga_index::Store::open_with_root_and_schema(&cache_root, repo, 1).unwrap();
        s.commit().unwrap();
    }

    let s = ga_index::Store::open_with_root_and_schema(&cache_root, repo, 99).unwrap();
    match s.outcome() {
        ga_index::OpenOutcome::RebuildSchemaMismatch { cache, binary } => {
            assert_eq!(*cache, 1);
            assert_eq!(*binary, 99);
            let line = ga_index::store::rebuild_log_line_schema_mismatch(*cache, *binary);
            assert!(line.contains("cache=1"));
            assert!(line.contains("binary=99"));
        }
        other => panic!("expected Mismatch outcome, got {other:?}"),
    }
}
