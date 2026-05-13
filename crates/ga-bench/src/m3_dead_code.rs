//! Dead-code UC scoring loop for the M3 gate.
//!
//! Builds GT via `Hd-ast` rule, runs `ga_dead_code(scope=None)`, scores
//! Precision (primary) + Recall + F1 against the expected_dead set
//! `(name, file)` derived from raw AST. Spec target: Precision ≥ 0.85.

use crate::gt_gen::hd_ast::HdAst;
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, ScoreOpts, SpecStatus};
use crate::BenchError;
use std::collections::{BTreeMap, BTreeSet};

/// Spec target — Precision ≥ 0.85 per Verification §.
pub const DEAD_CODE_SPEC_TARGET: f64 = 0.85;

pub fn score_dead_code(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let rule = HdAst;
    let gt_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&gt_store, &opts.fixture_dir)?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    // expected_dead = set of (file, name) from GT where expected_dead=true.
    let mut expected_dead: BTreeSet<(String, String)> = BTreeSet::new();
    for t in &tasks {
        let is_dead = t
            .query
            .get("expected_dead")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !is_dead {
            continue;
        }
        let name = t
            .query
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file = t
            .query
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || file.is_empty() {
            continue;
        }
        expected_dead.insert((file, name));
    }

    let mut rows = Vec::new();
    for retriever_name in &opts.retrievers {
        if retriever_name != "ga" {
            rows.push(deferred_row(retriever_name, opts));
            continue;
        }

        let cache = opts.cache_root.join(retriever_name);
        let _ = std::fs::remove_dir_all(&cache);
        let store = ga_index::Store::open_with_root(&cache, &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open store: {e}")))?;
        ga_query::indexer::build_index(&store, &opts.fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("build_index: {e}")))?;

        let t0 = std::time::Instant::now();
        let resp = ga_query::dead_code::dead_code(
            &store,
            &ga_query::dead_code::DeadCodeRequest::default(),
        )
        .map_err(|e| BenchError::Query(format!("dead_code: {e}")))?;
        let latency_ms = t0.elapsed().as_millis() as u64;

        let actual: BTreeSet<(String, String)> = resp
            .dead
            .iter()
            .map(|d| (d.file.clone(), d.symbol.clone()))
            .collect();

        // OpenAI/Codex review fix: ga's symbol indexer materialises a
        // narrower pool than parse_source emits (no nested closures /
        // inner fns). The bench's `expected_dead` includes those extras,
        // inflating FN with cases ga can't even consider candidates.
        // Standard SWE-bench retrieval move: intersect GT with the
        // engine's universe before scoring → apples-to-apples.
        let universe: BTreeSet<(String, String)> = ga_symbol_universe(&store)?;
        let raw_expected_count = expected_dead.len();
        let aligned_expected: BTreeSet<(String, String)> = expected_dead
            .iter()
            .filter(|pair| universe.contains(pair))
            .cloned()
            .collect();

        // Score on the aligned set — raw set surfaced as diagnostic only.
        let tp = aligned_expected.intersection(&actual).count() as f64;
        let fp = actual.difference(&aligned_expected).count() as f64;
        let fn_count = aligned_expected.difference(&actual).count() as f64;

        let precision = if tp + fp == 0.0 { 1.0 } else { tp / (tp + fp) };
        let recall = if tp + fn_count == 0.0 {
            1.0
        } else {
            tp / (tp + fn_count)
        };
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };

        let mut secondary = BTreeMap::new();
        secondary.insert("recall".to_string(), recall);
        secondary.insert("f1".to_string(), f1);
        secondary.insert(
            "expected_dead_aligned".to_string(),
            aligned_expected.len() as f64,
        );
        secondary.insert("expected_dead_raw".to_string(), raw_expected_count as f64);
        secondary.insert("ga_universe_size".to_string(), universe.len() as f64);
        secondary.insert("actual_dead_count".to_string(), actual.len() as f64);
        secondary.insert("true_positives".to_string(), tp);
        secondary.insert("false_positives".to_string(), fp);
        secondary.insert("false_negatives".to_string(), fn_count);

        let spec_status = if precision >= DEAD_CODE_SPEC_TARGET {
            SpecStatus::Pass
        } else {
            SpecStatus::Fail
        };

        rows.push(M3LeaderboardRow {
            retriever: retriever_name.clone(),
            fixture: opts.fixture_name.clone(),
            uc: "dead_code".to_string(),
            score: precision,
            secondary_metrics: secondary,
            spec_status,
            spec_target: DEAD_CODE_SPEC_TARGET,
            p95_latency_ms: latency_ms,
        });
    }

    Ok(rows)
}

/// OpenAI/Codex review fix: query ga's indexed symbol set so we can
/// intersect GT with what the engine actually considers a candidate.
/// Returns `(file, name)` tuples for non-external symbols — same shape
/// as the GT keys.
fn ga_symbol_universe(store: &ga_index::Store) -> Result<BTreeSet<(String, String)>, BenchError> {
    let conn = store
        .connection()
        .map_err(|e| BenchError::Query(format!("connection: {e}")))?;
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.file")
        .map_err(|e| BenchError::Query(format!("symbol universe query: {e}")))?;
    let mut out = BTreeSet::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };
        let file = match &cols[1] {
            lbug::Value::String(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };
        out.insert((file, name));
    }
    Ok(out)
}

fn deferred_row(retriever: &str, opts: &ScoreOpts) -> M3LeaderboardRow {
    let mut row = M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "dead_code".to_string(),
        score: 0.0,
        secondary_metrics: BTreeMap::new(),
        spec_status: SpecStatus::Deferred,
        spec_target: DEAD_CODE_SPEC_TARGET,
        p95_latency_ms: 0,
    };
    row.secondary_metrics
        .insert("note_competitor_adapter_pending".to_string(), 0.0);
    row
}
