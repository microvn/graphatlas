//! S-004 cycle 2 — AS-011.T2 sidecar verification.
//!
//! Per spec:
//! - GT generator must (1) read sidecar expected sha256, (2) compute actual
//!   sha256 of the input file, (3) panic with clear error on mismatch.
//!
//! Note: the panic-on-mismatch convention is a safety net for corrupted
//! input — git LFS dropouts, mid-merge state, etc. The error message must
//! point the user at remediation.

use ga_bench::m3_runner::{verify_sha256_sidecar, write_gt_atomic};
use tempfile::tempdir;

#[test]
fn as_011_t2_sidecar_verify_succeeds_when_hash_matches() {
    let dir = tempdir().unwrap();
    let payload = b"line1\nline2\n";
    let target = dir.path().join("dataset.jsonl");

    // Use the existing atomic-write helper (S-001) to produce a paired
    // file + sidecar; verify_sha256_sidecar reads the sidecar back.
    write_gt_atomic(&target, payload, 2).unwrap();

    verify_sha256_sidecar(&target).expect("matching sidecar must verify cleanly");
}

#[test]
fn as_011_t2_sidecar_verify_errors_when_hash_mismatches() {
    let dir = tempdir().unwrap();
    let payload = b"line1\nline2\n";
    let target = dir.path().join("dataset.jsonl");
    write_gt_atomic(&target, payload, 2).unwrap();

    // Tamper with the file after sidecar was written.
    std::fs::write(&target, b"line1\nline2\nINJECTED\n").unwrap();

    let err = verify_sha256_sidecar(&target).expect_err("mismatch must error");
    let msg = err.to_string();
    assert!(
        msg.contains("dataset.jsonl") || msg.contains(target.to_string_lossy().as_ref()),
        "error must name the offending file; got: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("mismatch") || msg.to_lowercase().contains("corrupt"),
        "error must use mismatch/corrupt terminology; got: {msg}"
    );
    assert!(
        msg.contains("git lfs") || msg.contains("merge") || msg.contains("regenerate"),
        "error must point at remediation (git lfs / merge conflicts / regen); got: {msg}"
    );
}

#[test]
fn as_011_t2_sidecar_verify_errors_when_sidecar_missing() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("dataset.jsonl");
    std::fs::write(&target, b"payload").unwrap();
    // No sidecar written.

    let err = verify_sha256_sidecar(&target).expect_err("missing sidecar must error");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("sidecar") || msg.to_lowercase().contains("missing"),
        "error must mention missing sidecar; got: {msg}"
    );
}
