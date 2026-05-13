//! S-001 cycle 2 — AS-003 atomic GT write + sha256 sidecar.
//!
//! Per spec:
//! - AS-003.T1: write to `<file>.tmp`, atomic rename to `<file>`.
//! - AS-003.T2: emit `<file>.sha256` sidecar with `{task_count, sha256}`.
//! - AS-003.T3: concurrent jobs serialize via tmp-rename — last writer wins,
//!   never partial JSON.

use ga_bench::m3_runner::write_gt_atomic;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

fn read_sidecar(path: &std::path::Path) -> serde_json::Value {
    let s = std::fs::read_to_string(path).expect("sidecar must exist");
    serde_json::from_str(&s).expect("sidecar must be valid JSON")
}

#[test]
fn as_003_t1_atomic_write_creates_final_file() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("preact.generated.json");
    let payload = br#"{"tasks":[{"id":"t1"}]}"#;

    write_gt_atomic(&target, payload, 1).expect("atomic write must succeed");

    assert!(target.is_file(), "final file must exist at {target:?}");
    let written = std::fs::read(&target).unwrap();
    assert_eq!(&written, payload, "file contents must match exactly");

    // No leftover tmp file
    let tmp = dir.path().join("preact.generated.json.tmp");
    assert!(!tmp.exists(), "tmp file must be cleaned up after rename");
}

#[test]
fn as_003_t2_sidecar_contains_task_count_and_sha256() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("axum.generated.json");
    let payload = br#"{"tasks":[{"id":"a"},{"id":"b"},{"id":"c"}]}"#;

    write_gt_atomic(&target, payload, 3).unwrap();

    let sidecar_path = dir.path().join("axum.generated.json.sha256");
    assert!(sidecar_path.is_file(), "sidecar must exist next to GT file");

    let sidecar = read_sidecar(&sidecar_path);
    assert_eq!(
        sidecar["task_count"].as_u64(),
        Some(3),
        "task_count field must reflect caller-supplied value; sidecar={sidecar}"
    );
    let hex = sidecar["sha256"]
        .as_str()
        .expect("sha256 field must be a hex string");
    assert_eq!(
        hex.len(),
        64,
        "sha256 must be 64-hex-char digest; got {hex}"
    );
    assert!(
        hex.chars().all(|c| c.is_ascii_hexdigit()),
        "sha256 must be hex-only; got {hex}"
    );

    // Expected sha256 of payload computed independently
    use sha2::{Digest, Sha256};
    let expected = ga_bench::manifest::hex_encode(&Sha256::digest(payload));
    assert_eq!(hex, expected, "sidecar sha256 must match content digest");
}

#[test]
fn as_003_t3_concurrent_writers_never_produce_partial_json() {
    let dir = tempdir().unwrap();
    let target = Arc::new(dir.path().join("gin.generated.json"));

    // Two payloads, both valid JSON. After both threads finish, the file must
    // contain exactly one of them — never an interleaved/partial blob.
    let payload_a = Arc::new(br#"{"tasks":[{"id":"writerA"}]}"#.to_vec());
    let payload_b = Arc::new(br#"{"tasks":[{"id":"writerB"}]}"#.to_vec());

    let t1 = {
        let target = Arc::clone(&target);
        let p = Arc::clone(&payload_a);
        thread::spawn(move || write_gt_atomic(&target, &p, 1))
    };
    let t2 = {
        let target = Arc::clone(&target);
        let p = Arc::clone(&payload_b);
        thread::spawn(move || write_gt_atomic(&target, &p, 1))
    };
    t1.join().unwrap().expect("writer A must succeed");
    t2.join().unwrap().expect("writer B must succeed");

    let final_bytes = std::fs::read(&*target).unwrap();
    assert!(
        final_bytes == *payload_a || final_bytes == *payload_b,
        "final file must equal one of the two payloads — never interleaved. got: {:?}",
        String::from_utf8_lossy(&final_bytes)
    );

    // Sidecar sha256 must match whichever payload won.
    let sidecar = read_sidecar(&dir.path().join("gin.generated.json.sha256"));
    use sha2::{Digest, Sha256};
    let hex_a = ga_bench::manifest::hex_encode(&Sha256::digest(payload_a.as_slice()));
    let hex_b = ga_bench::manifest::hex_encode(&Sha256::digest(payload_b.as_slice()));
    let actual_hex = sidecar["sha256"].as_str().unwrap().to_string();
    assert!(
        actual_hex == hex_a || actual_hex == hex_b,
        "sidecar sha256 must match the winning payload (got {actual_hex})"
    );
    let winning_payload = if actual_hex == hex_a {
        &*payload_a
    } else {
        &*payload_b
    };
    assert_eq!(
        &final_bytes, winning_payload,
        "sidecar sha256 must agree with file contents (race-safe write)"
    );
}
