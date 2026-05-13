//! M2 gate ground-truth schema (v2) — unified across 5 repos × 20 tasks.
//!
//! Produced by `scripts/mine-fix-commits.ts` + `scripts/extract-seeds.ts` +
//! `scripts/consolidate-gt.ts`. Stored at `benches/uc-impact/ground-truth.json`
//! with SHA256 sidecar at `ground-truth.sha256`. Loader verifies hash before
//! returning tasks so bench runs are reproducible across GT revisions.

use crate::manifest::verify_sha256;
use crate::BenchError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const EXPECTED_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    Dev,
    Test,
}

impl Split {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dev" => Some(Self::Dev),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct M2Task {
    pub task_id: String,
    pub repo: String,
    pub lang: String,
    pub base_commit: String,
    pub fix_commit: String,
    pub subject: String,
    pub seed_file: String,
    pub seed_symbol: String,
    pub source_files: Vec<String>,
    pub expected_files: Vec<String>,
    pub expected_tests: Vec<String>,
    /// Structural blast radius: source files that import/reference the seed
    /// module and historically co-change with it, but are NOT in expected_files.
    /// Derived offline by `scripts/extract-seeds.ts` via language-specific
    /// grep + co-change cross-validation (no GA graph — avoids circular bias).
    /// Empty when no importers+co-change intersection found. Schema v3+.
    #[serde(default)]
    pub should_touch_files: Vec<String>,
    #[serde(default)]
    pub max_expected_depth: Option<u32>,
    pub split: Split,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct M2GroundTruth {
    pub schema_version: u32,
    pub source: String,
    pub uc: String,
    pub spec: String,
    pub mining_tool: String,
    pub total_tasks: u32,
    pub per_repo: HashMap<String, u32>,
    pub per_lang: HashMap<String, u32>,
    pub tasks: Vec<M2Task>,
}

impl M2GroundTruth {
    /// Load + SHA256-verify. `data_path` must be `ground-truth.json`;
    /// sidecar `ground-truth.sha256` is expected in the same directory.
    pub fn load(data_path: &Path) -> Result<Self, BenchError> {
        let sidecar = data_path.with_extension("sha256");
        verify_sha256(data_path, &sidecar)?;
        let bytes = std::fs::read(data_path).map_err(|e| BenchError::GroundTruthMalformed {
            path: data_path.display().to_string(),
            reason: format!("read: {e}"),
        })?;
        let gt: Self =
            serde_json::from_slice(&bytes).map_err(|e| BenchError::GroundTruthMalformed {
                path: data_path.display().to_string(),
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

    /// Filter tasks by split. `None` = all splits.
    pub fn filter_split(&self, split: Option<Split>) -> Vec<&M2Task> {
        match split {
            Some(s) => self.tasks.iter().filter(|t| t.split == s).collect(),
            None => self.tasks.iter().collect(),
        }
    }

    /// Group tasks by repo for per-fixture processing.
    pub fn group_by_repo<'a>(tasks: &[&'a M2Task]) -> HashMap<String, Vec<&'a M2Task>> {
        let mut by_repo: HashMap<String, Vec<&M2Task>> = HashMap::new();
        for t in tasks {
            by_repo.entry(t.repo.clone()).or_default().push(*t);
        }
        by_repo
    }
}
