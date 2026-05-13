//! Real `ripgrep` subprocess retriever — per Bench-C3 the lexical baseline.
//!
//! Per-UC mapping (ripgrep has no AST, so only 2 UCs have a plausible answer):
//!   - importers    → `rg -l '<file-stem>'` → files containing the stem
//!   - symbols      → `rg -l '<pattern>'`   → if any match, return [pattern]
//!                      (MRR counts this as rank-1 hit)
//!   - callers / callees / file_summary → empty (ripgrep can't resolve these)
//!
//! This is deliberately the "no graph" baseline: we want the leaderboard to
//! surface GA's structural advantage, and an honest ripgrep can't fake it.
//!
//! Availability: `setup()` runs `which rg` and stores the result. If absent,
//! the retriever short-circuits all queries to empty — the leaderboard
//! records the entry with `pass_rate = 0` + the scorer emits the "N/A" row.
//! No panics, no crashes — CI without `rg` installed still completes.

use crate::retriever::Retriever;
use crate::BenchError;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct RipgrepRetriever {
    available: bool,
    fixture_dir: Option<PathBuf>,
}

impl Default for RipgrepRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl RipgrepRetriever {
    pub fn new() -> Self {
        Self {
            available: false,
            fixture_dir: None,
        }
    }
}

impl Retriever for RipgrepRetriever {
    fn name(&self) -> &str {
        "ripgrep"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_dir = Some(fixture_dir.to_path_buf());
        // Probe: `rg --version` is fast + portable availability check.
        self.available = Command::new("rg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        Ok(())
    }

    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        if !self.available {
            return Ok(Vec::new());
        }
        let Some(fixture_dir) = self.fixture_dir.as_ref() else {
            return Ok(Vec::new());
        };
        match uc {
            "importers" => {
                let Some(file) = query.get("file").and_then(|v| v.as_str()) else {
                    return Ok(Vec::new());
                };
                let stem = Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file);
                Ok(rg_list_files(fixture_dir, stem)
                    .into_iter()
                    .filter(|hit| hit != file)
                    .collect())
            }
            "symbols" => {
                let Some(pattern) = query.get("pattern").and_then(|v| v.as_str()) else {
                    return Ok(Vec::new());
                };
                // If any file in the fixture contains the literal pattern,
                // rank-1 the pattern itself (MRR floor). Otherwise empty.
                let hits = rg_list_files(fixture_dir, pattern);
                if hits.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![pattern.to_string()])
                }
            }
            "impact" => {
                // Plain-text fallback for M2 baseline: files containing the
                // seed symbol. No structure, no ranking — exactly the "floor"
                // retriever that GA should dominate if graphs add value.
                let Some(symbol) = query.get("symbol").and_then(|v| v.as_str()) else {
                    return Ok(Vec::new());
                };
                Ok(rg_list_files(fixture_dir, symbol))
            }
            // No plausible ripgrep answer for these — structural resolution
            // required. Returning empty surfaces GA's advantage on the
            // leaderboard (pass_rate will hit 0 for these UCs on rg).
            _ => Ok(Vec::new()),
        }
    }
}

/// `rg -l --no-messages --glob '!.git' --glob '!node_modules' <pattern> <dir>`
/// Returns repo-relative paths. Silently empty on non-zero exit (rg exits 1
/// when no matches — that's a normal "no hit", not an error).
fn rg_list_files(fixture_dir: &Path, pattern: &str) -> Vec<String> {
    let out = Command::new("rg")
        .args([
            "-l",
            "--no-messages",
            "--glob",
            "!.git",
            "--glob",
            "!node_modules",
            "--glob",
            "!.cache",
            pattern,
        ])
        .arg(fixture_dir)
        .output();
    let Ok(out) = out else {
        return Vec::new();
    };
    let prefix = format!("{}/", fixture_dir.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.strip_prefix(&prefix).unwrap_or(l).to_string())
        .collect()
}
