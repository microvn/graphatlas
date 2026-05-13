//! S-001 ga_risk — standalone risk composite (Tools-C2 pinned formula).
//!
//! Spec contract (graphatlas-v1.1-tools.md S-001):
//!   `0.4·test_gap + 0.3·blast_radius + 0.15·blame_churn + 0.15·bug_correlation`
//!
//! Reuses [`ga_query::impact`] for test_gap + blast_radius computation
//! (already shipped in v1) and adds blame-mined dims via the [`BlameMiner`]
//! abstraction so callers can inject a stub miner in tests.
//!
//! Differences from the embedded `impact::risk` (intentional, see
//! Tools-C2 ADR-pending):
//! - Spec formula replaces `depth + exposure` with `blame_churn + bug_correlation`
//!   (research-backed predictors per Hassan-Holt 2005 / MSR Zimmermann-Nagappan).
//! - AS-002 fresh-symbol carve-out: `test_gap` clamped to neutral 0.5
//!   when the seed has 0 callers (avoids false-high on deletions).
//! - AS-003 changed_files mode returns `meta.per_file: {file: score}`
//!   breakdown alongside max-per-file `score`.
//! - AS-004 unknown-symbol error path returns `Error::InvalidParams` with
//!   top-3 Levenshtein suggestions baked into the message — `ga-mcp`
//!   maps to JSON-RPC `-32602`.

use crate::blame::{BlameMiner, BlameStats};
use crate::common::levenshtein;
use crate::impact::{self, ImpactRequest, ImpactResponse};
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Saturation: ≥20 callers → blast = 1.0 (matches `impact::risk` constant).
const BLAST_SATURATION: f32 = 20.0;
/// Lookback window for blame mining (AS-001 §Data: "last 90 days").
const BLAME_WINDOW_DAYS: u32 = 90;
/// Levenshtein suggestion cap for AS-004 not-found error.
const SUGGESTION_LIMIT: usize = 3;

/// Composite weights — Tools-C2 PINNED. Modifying requires ADR + test split
/// validation per Tools-C2.
const W_TEST_GAP: f32 = 0.4;
const W_BLAST: f32 = 0.3;
const W_CHURN: f32 = 0.15;
const W_BUG: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerDim {
    pub test_gap: f32,
    pub blast_radius: f32,
    pub blame_churn: f32,
    pub bug_correlation: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RiskMeta {
    pub per_dim: PerDim,
    /// AS-003: per-file breakdown for changed_files mode. Empty in
    /// symbol mode.
    pub per_file: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskResponse {
    pub score: f32,
    pub level: RiskLevel,
    pub reasons: Vec<String>,
    pub meta: RiskMeta,
    /// v1.4 S-001d / AS-014 — override chain depth from this symbol up the
    /// OVERRIDES chain to the root. `Some(0)` for non-override symbols /
    /// override chain roots. `Some(N)` for an N-deep chain. `None` only
    /// when symbol mode wasn't entered (changed_files mode). **Wiring
    /// only** — score formula behavior is NOT pinned in v1.4 (deferred
    /// to a future EXP-RISK-OVERRIDE-WEIGHT entry under EXPERIMENTS.md
    /// gated on real fixture leaderboard data).
    #[serde(default)]
    pub override_chain_depth: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct RiskRequest {
    pub symbol: Option<String>,
    pub file_hint: Option<String>,
    pub changed_files: Option<Vec<String>>,
    /// Optional anchor commit (`ref` resolvable by `git rev-parse`) for
    /// time-window mining. When set, BlameStats uses commits in the 90
    /// days BEFORE this commit's committer-date instead of wall-clock.
    /// Production `ga_risk` (MCP) leaves this `None` — wall-clock is the
    /// right semantics for live repos. M3 bench harness sets it to
    /// fixture HEAD so engine and GT mining share the same time anchor.
    pub anchor_ref: Option<String>,
}

impl RiskRequest {
    pub fn for_symbol(symbol: impl Into<String>) -> Self {
        Self {
            symbol: Some(symbol.into()),
            ..Default::default()
        }
    }
    pub fn for_changed_files(files: Vec<String>) -> Self {
        Self {
            changed_files: Some(files),
            ..Default::default()
        }
    }
    /// Bench helper — set anchor on top of an existing constructor.
    pub fn with_anchor(mut self, anchor_ref: impl Into<String>) -> Self {
        self.anchor_ref = Some(anchor_ref.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────

pub fn risk<M: BlameMiner>(store: &Store, miner: &M, req: &RiskRequest) -> Result<RiskResponse> {
    let symbol = req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let changed = req.changed_files.as_deref().filter(|v| !v.is_empty());

    let anchor = req.anchor_ref.as_deref();
    match (symbol, changed) {
        (Some(sym), _) => risk_for_symbol(store, miner, sym, req.file_hint.as_deref(), anchor),
        (None, Some(files)) => risk_for_changed_files(store, miner, files, anchor),
        (None, None) => Err(Error::InvalidParams(
            "ga_risk: at least one of `symbol` or `changed_files` required".to_string(),
        )),
    }
}

/// Helper: pick `compute_in_window` when bench passes anchor, else
/// fall back to wall-clock `compute` for production semantics.
fn blame_for_file<M: BlameMiner>(
    miner: &M,
    file: &str,
    days: u32,
    anchor: Option<&str>,
) -> BlameStats {
    match anchor {
        Some(a) => BlameStats::compute_in_window(miner, file, a, days),
        None => BlameStats::compute(miner, file, days),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Symbol mode (AS-001, AS-002, AS-004)
// ─────────────────────────────────────────────────────────────────────────

fn risk_for_symbol<M: BlameMiner>(
    store: &Store,
    miner: &M,
    symbol: &str,
    file_hint: Option<&str>,
    anchor: Option<&str>,
) -> Result<RiskResponse> {
    // Resolve symbol → impact signals. ga_query::impact handles AS-014
    // qualified-seed expansion + Tools-C9-d ident allowlist + same-file
    // resolution path internally.
    let impact_req = ImpactRequest {
        symbol: Some(symbol.to_string()),
        file: file_hint.map(str::to_string),
        changed_files: None,
        diff: None,
        ..Default::default()
    };
    let impact_resp = impact::impact(store, &impact_req)?;

    // AS-004: empty impact result on a non-empty input means symbol not
    // found — emit InvalidParams with Levenshtein suggestions.
    if impact_resp.impacted_files.is_empty() && impact_resp.break_points.is_empty() {
        let suggestions = nearest_symbol_names(store, symbol)?;
        return Err(Error::SymbolNotFound { suggestions });
    }

    let seed_file = impact_resp
        .impacted_files
        .first()
        .map(|f| f.path.clone())
        .or_else(|| impact_resp.break_points.first().map(|b| b.file.clone()))
        .unwrap_or_default();
    let blame = blame_for_file(miner, &seed_file, BLAME_WINDOW_DAYS, anchor);

    let dims = compose_dims(&impact_resp, &blame);
    let score = compose_score(
        dims.test_gap,
        dims.blast_radius,
        dims.blame_churn,
        dims.bug_correlation,
    );
    let level = level_from_score(score);
    let reasons = reasons_for(&impact_resp, &blame, &dims);

    // v1.4 S-001d / AS-014 — populate override_chain_depth (wiring only).
    let override_chain_depth = compute_override_chain_depth(store, symbol, file_hint).ok();

    Ok(RiskResponse {
        score,
        level,
        reasons,
        meta: RiskMeta {
            per_dim: dims,
            per_file: BTreeMap::new(),
        },
        override_chain_depth,
    })
}

/// v1.4 S-001d / AS-014 — walk the OVERRIDES chain from `symbol` up to its
/// root, returning the depth (0 for non-override / chain root, N for N-deep
/// chain). Implementation: iterative single-step Cypher hops capped at a
/// safety bound (10 levels) to avoid pathological cycles. Tools-C19 single-
/// step OVERRIDES emission means we walk one hop at a time rather than
/// relying on `OVERRIDES*` Kleene path support in lbug.
fn compute_override_chain_depth(
    store: &Store,
    symbol: &str,
    file_hint: Option<&str>,
) -> Result<i64> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let safe_name = symbol.replace('\'', "");
    // Resolve current symbol's file: file_hint when provided, else first
    // matching def's file (multi-def is rare for OO method symbols).
    let mut cur_name = safe_name.clone();
    let mut cur_file = match file_hint {
        Some(f) => f.replace('\'', ""),
        None => {
            // Look up first definition file.
            let q = format!(
                "MATCH (s:Symbol {{name: '{cur_name}'}}) WHERE s.kind <> 'external' \
                 RETURN s.file LIMIT 1"
            );
            let mut found = String::new();
            if let Ok(rs) = conn.query(&q) {
                for row in rs {
                    if let Some(lbug::Value::String(f)) = row.into_iter().next() {
                        found = f;
                        break;
                    }
                }
            }
            found
        }
    };
    if cur_file.is_empty() {
        return Ok(0);
    }
    let mut depth: i64 = 0;
    for _ in 0..10 {
        // Find immediate parent via OVERRIDES.
        let q = format!(
            "MATCH (c:Symbol {{name: '{cur_name}', file: '{cur_file}'}})-[:OVERRIDES]->(p:Symbol) \
             RETURN p.name, p.file LIMIT 1"
        );
        let mut next: Option<(String, String)> = None;
        if let Ok(rs) = conn.query(&q) {
            for row in rs {
                let cols: Vec<lbug::Value> = row.into_iter().collect();
                if cols.len() >= 2 {
                    if let (lbug::Value::String(pn), lbug::Value::String(pf)) = (&cols[0], &cols[1])
                    {
                        next = Some((pn.clone(), pf.clone()));
                        break;
                    }
                }
            }
        }
        match next {
            Some((pn, pf)) => {
                depth += 1;
                cur_name = pn;
                cur_file = pf;
            }
            None => break,
        }
    }
    Ok(depth)
}

// ─────────────────────────────────────────────────────────────────────────
// changed_files mode (AS-003)
// ─────────────────────────────────────────────────────────────────────────

fn risk_for_changed_files<M: BlameMiner>(
    store: &Store,
    miner: &M,
    files: &[String],
    anchor: Option<&str>,
) -> Result<RiskResponse> {
    let mut per_file: BTreeMap<String, f32> = BTreeMap::new();
    let mut max_dims = PerDim::default();
    let mut max_score = 0.0f32;

    for file in files {
        let impact_req = ImpactRequest {
            changed_files: Some(vec![file.clone()]),
            ..Default::default()
        };
        let impact_resp = impact::impact(store, &impact_req)?;
        let blame = blame_for_file(miner, file, BLAME_WINDOW_DAYS, anchor);
        let dims = compose_dims(&impact_resp, &blame);
        let score = compose_score(
            dims.test_gap,
            dims.blast_radius,
            dims.blame_churn,
            dims.bug_correlation,
        );
        per_file.insert(file.clone(), score);
        if score > max_score {
            max_score = score;
            max_dims = dims;
        }
    }

    let reasons = if max_score == 0.0 {
        Vec::new()
    } else {
        // Synthesize a per-file reason summary for the top-scoring file.
        let top = per_file
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(f, _)| f.clone())
            .unwrap_or_default();
        vec![format!("max risk concentrated in `{top}`")]
    };

    Ok(RiskResponse {
        score: max_score,
        level: level_from_score(max_score),
        reasons,
        meta: RiskMeta {
            per_dim: max_dims,
            per_file,
        },
        // changed_files mode is per-file aggregation; override_chain_depth
        // is symbol-level. Surface as None so consumers can distinguish
        // "didn't compute" from "depth=0".
        override_chain_depth: None,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Composition helpers
// ─────────────────────────────────────────────────────────────────────────

fn compose_dims(impact_resp: &ImpactResponse, blame: &BlameStats) -> PerDim {
    // No signal at all (unknown file, unknown symbol slipping through) →
    // all-zero dims so score = 0. Distinguishes from AS-002 "fresh symbol
    // with 0 callers" where impacted_files contains the seed at depth 0.
    if impact_resp.impacted_files.is_empty()
        && impact_resp.break_points.is_empty()
        && blame.commit_count == 0
    {
        return PerDim::default();
    }

    let break_count = impact_resp.break_points.len();
    let test_count = impact_resp.affected_tests.len();
    // AS-002 carve-out: fresh symbol (callers=0) → neutral test_gap.
    // Only fires when the seed itself was found (impacted_files non-empty
    // OR blame history exists) — pure unknown is handled by the early
    // return above.
    let test_gap = if break_count == 0 {
        0.5
    } else {
        let ratio = test_count as f32 / break_count as f32;
        (1.0 - ratio).clamp(0.0, 1.0)
    };
    let impacted = impact_resp.impacted_files.len() as f32;
    let blast_radius = (impacted / BLAST_SATURATION).min(1.0);

    PerDim {
        test_gap,
        blast_radius,
        blame_churn: blame.churn(),
        bug_correlation: blame.bug_correlation(),
    }
}

/// Pure compose function — exposed for Tools-C2 weight regression test.
pub fn compose_score(
    test_gap: f32,
    blast_radius: f32,
    blame_churn: f32,
    bug_correlation: f32,
) -> f32 {
    (W_TEST_GAP * test_gap.clamp(0.0, 1.0)
        + W_BLAST * blast_radius.clamp(0.0, 1.0)
        + W_CHURN * blame_churn.clamp(0.0, 1.0)
        + W_BUG * bug_correlation.clamp(0.0, 1.0))
    .clamp(0.0, 1.0)
}

fn level_from_score(score: f32) -> RiskLevel {
    if score >= 0.7 {
        RiskLevel::High
    } else if score >= 0.4 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    }
}

fn reasons_for(impact_resp: &ImpactResponse, blame: &BlameStats, dims: &PerDim) -> Vec<String> {
    let mut contribs: Vec<(f32, String)> = Vec::new();

    if dims.test_gap > 0.0 && impact_resp.break_points.len() > 0 {
        let untested = impact_resp.break_points.len()
            - impact_resp
                .affected_tests
                .len()
                .min(impact_resp.break_points.len());
        let total = impact_resp.break_points.len();
        let files = impact_resp
            .impacted_files
            .iter()
            .map(|f| f.path.clone())
            .collect::<std::collections::HashSet<_>>()
            .len();
        contribs.push((
            W_TEST_GAP * dims.test_gap,
            format!(
                "{total} caller{} in {files} file{} ({} untested)",
                if total == 1 { "" } else { "s" },
                if files == 1 { "" } else { "s" },
                untested
            ),
        ));
    }
    if dims.blast_radius > 0.0 {
        contribs.push((
            W_BLAST * dims.blast_radius,
            format!(
                "blast: {} impacted file(s)",
                impact_resp.impacted_files.len()
            ),
        ));
    }
    if blame.commit_count > 0 {
        contribs.push((
            W_CHURN * dims.blame_churn,
            format!("churn: {} commits/90d", blame.commit_count),
        ));
    }
    if blame.bug_fix_count > 0 {
        contribs.push((
            W_BUG * dims.bug_correlation,
            format!(
                "{} bug-fix commit{} touched this symbol",
                blame.bug_fix_count,
                if blame.bug_fix_count == 1 { "" } else { "s" }
            ),
        ));
    }
    if contribs.is_empty() {
        // AS-002: explicit fresh-symbol reasons.
        if impact_resp.break_points.is_empty() {
            return vec![
                "0 callers (fresh symbol)".to_string(),
                "no git history".to_string(),
            ];
        }
    }
    contribs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    contribs.into_iter().take(3).map(|(_, r)| r).collect()
}

// ─────────────────────────────────────────────────────────────────────────
// AS-004 — Levenshtein suggestions for unknown symbol
// ─────────────────────────────────────────────────────────────────────────

fn nearest_symbol_names(store: &Store, target: &str) -> Result<Vec<String>> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN DISTINCT s.name")
        .map_err(|e| Error::Other(anyhow::anyhow!("nearest_symbol_names: {e}")))?;

    let mut scored: Vec<(u32, String)> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(name)) = row.into_iter().next() {
            let d = levenshtein(target, &name);
            scored.push((d, name));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(scored
        .into_iter()
        .take(SUGGESTION_LIMIT)
        .map(|(_, n)| n)
        .collect())
}
