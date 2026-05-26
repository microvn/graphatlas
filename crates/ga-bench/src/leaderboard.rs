//! Markdown leaderboard writer. Output shape pinned by AS-001:
//! `retriever | F1 | Recall | p95 latency | pass rate`, with a "Hardware:"
//! footer line so external reviewers know the baseline.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderEntry {
    pub retriever: String,
    pub f1: f64,
    /// F2 score (recall weighted 2× precision) — recall-priority lens for
    /// graph-retriever value-add. Defaults to 0 for legacy entries written
    /// before the metric existed; the 2026-05-23 v2 bench populates it
    /// alongside f1 for callers/callees/importers/file_summary UCs.
    #[serde(default)]
    pub f2: f64,
    pub recall: f64,
    pub precision: f64,
    /// MRR populated for ranked UCs (symbols). 0.0 when N/A.
    #[serde(default)]
    pub mrr: f64,
    pub p95_latency_ms: u64,
    /// Fraction of tasks where the retriever returned any result ≥ expected's F1 floor.
    pub pass_rate: f64,
    /// Mean tokens carried in the MCP-shape response across all tasks.
    /// `chars(serialized) / 4` per task, averaged. Distinct from M2's
    /// file-read cost — this is what the agent sees as direct response payload.
    /// Defaults to 0 for legacy entries written before the metric existed.
    #[serde(default)]
    pub payload_tokens_mean: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leaderboard {
    pub uc: String,
    pub fixture: String,
    pub hardware: String,
    pub entries: Vec<LeaderEntry>,
}

/// Render the leaderboard as Markdown into `out`. Sorted by F1 descending
/// (ties broken by retriever name for deterministic output).
pub fn write_leaderboard(lb: &Leaderboard, out: &Path) -> std::io::Result<()> {
    let mut entries = lb.entries.clone();
    entries.sort_by(|a, b| {
        b.f1.partial_cmp(&a.f1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.retriever.cmp(&b.retriever))
    });

    let mut f = std::fs::File::create(out)?;
    writeln!(f, "# Leaderboard: UC `{}`", lb.uc)?;
    writeln!(f)?;
    writeln!(f, "**Fixture:** {}", lb.fixture)?;
    writeln!(f, "**Hardware:** {}", lb.hardware)?;
    writeln!(f)?;
    writeln!(
        f,
        "| Retriever | F1 | F2 | Recall | Precision | MRR | p95 latency | pass rate | payload tokens |"
    )?;
    writeln!(
        f,
        "|-----------|----|----|--------|-----------|-----|-------------|-----------|----------------|"
    )?;
    for e in &entries {
        writeln!(
            f,
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {} ms | {:.1}% | {:.0} |",
            e.retriever,
            e.f1,
            e.f2,
            e.recall,
            e.precision,
            e.mrr,
            e.p95_latency_ms,
            e.pass_rate * 100.0,
            e.payload_tokens_mean,
        )?;
    }
    Ok(())
}
