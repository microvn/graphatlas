//! M1 gate ground-truth schema (uc-callers, uc-importers).
//!
//! M2 (uc-impact) uses [`crate::m2_ground_truth::M2GroundTruth`] — a v3 flat
//! schema with `seed_symbol` / `seed_file` / `should_touch_files` derived from
//! git mining. M3 mines GT at runtime via [`crate::gt_gen::hmc_gitmine`] and
//! does not use any static struct.
//!
//! Stored at `benches/uc-<callers|importers>/<fixture>.json`, committed per
//! Bench-C1 (offline, no runtime generation).

use crate::BenchError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Bench-C1 — GT schema version. Mismatch → fail fast with migration hint
/// (AS-003). Bump alongside any schema-breaking field change.
pub const EXPECTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct M1GroundTruth {
    pub schema_version: u32,
    pub uc: String,
    pub fixture: String,
    pub tasks: Vec<M1Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct M1Task {
    pub task_id: String,
    /// Query target — shape depends on UC:
    /// callers/callees: "symbol" (+ optional "file")
    /// importers/file_summary: "file"
    /// symbols: "pattern" (+ optional "match")
    pub query: serde_json::Value,
    /// Expected result set. For MRR-scored UCs this list is the single
    /// ground-truth target; for F1-scored UCs it's the full expected set.
    pub expected: Vec<String>,
}

impl M1GroundTruth {
    /// Load from disk + validate schema version (AS-003).
    pub fn load(path: &Path) -> Result<Self, BenchError> {
        let bytes = std::fs::read(path).map_err(|e| BenchError::GroundTruthMalformed {
            path: path.display().to_string(),
            reason: format!("read: {e}"),
        })?;
        let gt: M1GroundTruth =
            serde_json::from_slice(&bytes).map_err(|e| BenchError::GroundTruthMalformed {
                path: path.display().to_string(),
                reason: format!("parse: {e}"),
            })?;
        if gt.schema_version != EXPECTED_SCHEMA_VERSION {
            return Err(BenchError::SchemaMismatch {
                got: gt.schema_version,
                expected: EXPECTED_SCHEMA_VERSION,
            });
        }
        Ok(gt)
    }
}
