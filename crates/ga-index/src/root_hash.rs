//! v1.5 PR2 foundation S-001 AS-001 — populate `indexed_root_hash`.
//!
//! Extracted from `store.rs` so the Merkle-anchoring logic is testable in
//! isolation and store.rs stays focused on lifecycle. The contract is
//! strict (AS-001): `compute_root_hash` errors propagate, no graceful
//! degrade. Tests using synthetic paths must materialize real fixture dirs.

use crate::metadata::Metadata;
use ga_core::Result;
use std::path::Path;

/// Compute the bounded Merkle root hash of the indexed repo and store
/// its hex form on the `Metadata`. Called from `commit` and
/// `commit_in_place` immediately before the on-disk write.
///
/// Uses `ga_parser::merkle::compute_root_hash` with default `MerkleConfig`
/// (N≤32 dirs at depth ≤2 + `.git/index` mtime + `.git/HEAD` content)
/// per Foundation-C9 bounded contract.
pub(crate) fn populate_root_hash(metadata: &mut Metadata) -> Result<()> {
    let repo_root = Path::new(&metadata.repo_root);
    let cfg = ga_parser::merkle::MerkleConfig::default();
    let bytes = ga_parser::merkle::compute_root_hash(repo_root, &cfg)?;
    metadata.indexed_root_hash = hex_lower(&bytes);
    Ok(())
}

/// Lower-case hex encoding for the 32-byte BLAKE3 root hash. 64 chars out.
/// Inlined so `ga-index` does not need a `hex` crate dependency just for
/// this single use.
pub(crate) fn hex_lower(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
