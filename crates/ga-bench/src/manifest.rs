//! SHA256 manifest verification for M2 gate datasets.
//!
//! The M2 ground truth (`benches/uc-impact/ground-truth.json`) is committed
//! alongside its SHA256 sidecar (`ground-truth.sha256`). Bench runner verifies
//! hash before reading the dataset, aborting with a clear error if they drift.
//! Prevents silent GT edits that would invalidate historical leaderboard runs.
//!
//! Sidecar format is compatible with `shasum -a 256 -c`:
//! ```text
//! <64-hex-digest>  <filename>
//! ```

use crate::BenchError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Compute SHA256 of `data_path` and compare to the hex digest stored in
/// `sidecar_path`. Returns `Ok(())` if they match, otherwise a clear error
/// with both digests so the user can diagnose.
pub fn verify_sha256(data_path: &Path, sidecar_path: &Path) -> Result<(), BenchError> {
    let data = fs::read(data_path).map_err(|e| {
        BenchError::Other(anyhow::anyhow!(
            "manifest: read {}: {e}",
            data_path.display()
        ))
    })?;
    let sidecar = fs::read_to_string(sidecar_path).map_err(|e| {
        BenchError::Other(anyhow::anyhow!(
            "manifest: read {}: {e}",
            sidecar_path.display()
        ))
    })?;

    // Parse sidecar: "<hex>  <filename>\n"
    let expected_hex = sidecar
        .split_whitespace()
        .next()
        .ok_or_else(|| {
            BenchError::Other(anyhow::anyhow!(
                "manifest: sidecar {} is empty",
                sidecar_path.display()
            ))
        })?
        .to_lowercase();

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual_hex = hex_encode(&hasher.finalize());

    if actual_hex != expected_hex {
        return Err(BenchError::Other(anyhow::anyhow!(
            "manifest hash mismatch for {}:\n  expected: {}\n  actual:   {}\n\
             refuse to run bench — dataset drift detected",
            data_path.display(),
            expected_hex,
            actual_hex,
        )));
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn verify_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data.json");
        let sc = tmp.path().join("data.sha256");
        fs::write(&data, b"hello world\n").unwrap();
        // shasum -a 256 of "hello world\n" = a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447
        let mut f = fs::File::create(&sc).unwrap();
        writeln!(
            f,
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447  data.json"
        )
        .unwrap();
        verify_sha256(&data, &sc).unwrap();
    }

    #[test]
    fn verify_detects_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data.json");
        let sc = tmp.path().join("data.sha256");
        fs::write(&data, b"hello world\n").unwrap();
        fs::write(
            &sc,
            "0000000000000000000000000000000000000000000000000000000000000000  data.json\n",
        )
        .unwrap();
        let err = verify_sha256(&data, &sc).unwrap_err();
        assert!(err.to_string().contains("mismatch"));
    }
}
