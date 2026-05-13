//! Rename-safety UC scoring loop for the M3 gate.
//!
//! For each Hrn-static GT task, run `ga_rename_safety(target, file_hint)`,
//! compare actual sites vs `expected_sites`, then aggregate into per-tier
//! recall (unique vs polymorphic). Spec target: unique ≥ 0.90 AND
//! polymorphic ≥ 0.70 — both must hold for PASS.

use crate::gt_gen::hrn_static::HrnStatic;
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, ScoreOpts, SpecStatus};
use crate::BenchError;
use std::collections::{BTreeMap, BTreeSet};

pub const RENAME_UNIQUE_TARGET: f64 = 0.90;
pub const RENAME_POLY_TARGET: f64 = 0.70;
/// Cap targets-per-fixture. Each call to `ga_rename_safety` walks the
/// graph for callers — O(symbols) per call. django (~500k symbols)
/// at cap=50 still timed out >8 min; cap=20 gives 2-3 min. Stratified
/// sample (poly first, then unique) keeps both tiers represented.
const MAX_TARGETS_TO_SCORE: usize = 20;
const MAX_POLY: usize = 10;

pub fn score_rename_safety(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let rule = HrnStatic;
    let gt_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&gt_store, &opts.fixture_dir)?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    // Sample tasks: prioritise polymorphic (harder tier) up to MAX_POLY,
    // fill remainder with unique. Avoids fanning out to 10k calls on
    // django-scale fixtures.
    let (mut poly_pool, mut unique_pool): (Vec<&_>, Vec<&_>) = tasks
        .iter()
        .partition(|t| t.query.get("def_kind").and_then(|v| v.as_str()) == Some("polymorphic"));
    poly_pool.truncate(MAX_POLY);
    let unique_budget = MAX_TARGETS_TO_SCORE.saturating_sub(poly_pool.len());
    unique_pool.truncate(unique_budget);
    let sampled: Vec<&_> = poly_pool.into_iter().chain(unique_pool).collect();

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

        let mut unique_recalls: Vec<f64> = Vec::new();
        let mut poly_recalls: Vec<f64> = Vec::new();
        let mut latencies_ms: Vec<u64> = Vec::new();

        for task in &sampled {
            let target = task
                .query
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if target.is_empty() {
                continue;
            }
            let file_hint = task
                .query
                .get("file")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let def_kind = task
                .query
                .get("def_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("unique");

            let expected: BTreeSet<(String, u32)> = task
                .query
                .get("expected_sites")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| {
                            let f = s.get("file").and_then(|v| v.as_str())?;
                            let l = s.get("line").and_then(|v| v.as_u64())?;
                            Some((f.to_string(), l as u32))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if expected.is_empty() {
                continue;
            }

            let req = ga_query::rename_safety::RenameSafetyRequest {
                target: target.to_string(),
                replacement: format!("{target}_renamed"),
                file_hint,
                new_arity: None,
            };
            let t0 = std::time::Instant::now();
            let resp = match ga_query::rename_safety::rename_safety(&store, &req) {
                Ok(r) => r,
                Err(_) => {
                    // Polymorphic-without-hint or unknown target ⇒ recall 0.
                    if def_kind == "polymorphic" {
                        poly_recalls.push(0.0);
                    } else {
                        unique_recalls.push(0.0);
                    }
                    latencies_ms.push(t0.elapsed().as_millis() as u64);
                    continue;
                }
            };
            latencies_ms.push(t0.elapsed().as_millis() as u64);

            let actual: BTreeSet<(String, u32)> = resp
                .sites
                .iter()
                .map(|s| (s.file.clone(), s.line))
                .collect();
            let hits = expected.intersection(&actual).count() as f64;
            let recall = hits / expected.len() as f64;
            if def_kind == "polymorphic" {
                poly_recalls.push(recall);
            } else {
                unique_recalls.push(recall);
            }
        }

        let mean = |xs: &[f64]| -> f64 {
            if xs.is_empty() {
                1.0 // vacuous tier ⇒ pass-through (no targets in this tier)
            } else {
                xs.iter().sum::<f64>() / xs.len() as f64
            }
        };
        let recall_unique = mean(&unique_recalls);
        let recall_poly = mean(&poly_recalls);
        // Composite "score" = min of the two tier recalls (both must clear
        // their target). This mirrors PASS = recall_unique ≥ 0.90 AND
        // recall_poly ≥ 0.70.
        let composite = recall_unique.min(recall_poly);

        latencies_ms.sort_unstable();
        let p95_idx = if latencies_ms.is_empty() {
            0
        } else {
            ((latencies_ms.len() as f64) * 0.95).ceil() as usize - 1
        };
        let p95 = latencies_ms.get(p95_idx).copied().unwrap_or(0);

        let unique_pass = recall_unique >= RENAME_UNIQUE_TARGET;
        let poly_pass = recall_poly >= RENAME_POLY_TARGET;
        let spec_status = if unique_pass && poly_pass {
            SpecStatus::Pass
        } else {
            SpecStatus::Fail
        };

        let mut secondary = BTreeMap::new();
        secondary.insert("recall_unique".to_string(), recall_unique);
        secondary.insert("recall_polymorphic".to_string(), recall_poly);
        secondary.insert(
            "unique_target_count".to_string(),
            unique_recalls.len() as f64,
        );
        secondary.insert("poly_target_count".to_string(), poly_recalls.len() as f64);
        // Composite is reported as `score`; spec_target reports the
        // unique-tier 0.90 (the stricter of the two); poly tier surfaced
        // via secondary.
        secondary.insert("spec_target_polymorphic".to_string(), RENAME_POLY_TARGET);

        rows.push(M3LeaderboardRow {
            retriever: retriever_name.clone(),
            fixture: opts.fixture_name.clone(),
            uc: "rename_safety".to_string(),
            score: composite,
            secondary_metrics: secondary,
            spec_status,
            spec_target: RENAME_UNIQUE_TARGET,
            p95_latency_ms: p95,
        });
    }
    Ok(rows)
}

fn deferred_row(retriever: &str, opts: &ScoreOpts) -> M3LeaderboardRow {
    let mut row = M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "rename_safety".to_string(),
        score: 0.0,
        secondary_metrics: BTreeMap::new(),
        spec_status: SpecStatus::Deferred,
        spec_target: RENAME_UNIQUE_TARGET,
        p95_latency_ms: 0,
    };
    row.secondary_metrics
        .insert("note_competitor_adapter_pending".to_string(), 0.0);
    row
}
