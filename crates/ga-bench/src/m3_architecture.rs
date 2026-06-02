//! Architecture UC scoring loop for the M3 gate.
//!
//! Compares `ga_architecture(...)`'s edge set with the Ha-import-edge GT
//! edges. Primary metric: F1 on edge-pairs (Spearman utility deferred to
//! Phase 3 EXP per spec §"Not in Scope" + AS-019). Spec target: ≥ 0.6.

use crate::gt_gen::ha_import_edge::HaImportEdge;
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, ScoreOpts, SpecStatus};
use crate::BenchError;
use std::collections::{BTreeMap, BTreeSet};

pub const ARCHITECTURE_SPEC_TARGET: f64 = 0.6;

pub fn score_architecture(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let rule = HaImportEdge;
    let gt_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&gt_store, &opts.fixture_dir)?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    // Expected edge set from GT — only `kind=edge` tasks; `kind=module`
    // entries are diagnostic (per AS-019.T3). Also collect file_pair_count
    // per edge for the Spearman rank correlation (AS-019.T2 primary metric
    // post Codex review — F1 fallback used when ranks are tied / sample
    // size is too small for Spearman to be meaningful).
    let mut expected_edges: BTreeSet<(String, String)> = BTreeSet::new();
    let mut expected_weights: BTreeMap<(String, String), u32> = BTreeMap::new();
    for t in &tasks {
        if t.query.get("kind").and_then(|v| v.as_str()) != Some("edge") {
            continue;
        }
        let a = t
            .query
            .get("module_a")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let b = t
            .query
            .get("module_b")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let count = t
            .query
            .get("file_pair_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if !a.is_empty() && !b.is_empty() {
            let key = (a.to_string(), b.to_string());
            expected_edges.insert(key.clone());
            expected_weights.insert(key, count);
        }
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
        let resp = ga_query::architecture::architecture(
            &store,
            &ga_query::architecture::ArchitectureRequest::default(),
        )
        .map_err(|e| BenchError::Query(format!("architecture: {e}")))?;
        let latency_ms = t0.elapsed().as_millis() as u64;

        // Per-language consistency (C2): match the actual edge-kind set to the
        // GT's authority so we compare like with like. Rust workspaces (Cargo
        // manifest GT = module dependency graph) → KIND-AGNOSTIC actual
        // (deps surface as CALLS/EXTENDS, not imports). Everything else (Python
        // import-resolved GT) → imports-only actual, as before. A global
        // kind-agnostic comparison would flood the Python actual set with
        // calls/extends the import-GT never had and crater precision.
        let kind_agnostic = opts.fixture_dir.join("Cargo.toml").is_file();
        let edge_in_scope = |k: &str| kind_agnostic || k == "imports";
        let actual: BTreeSet<(String, String)> = resp
            .edges
            .iter()
            .filter(|e| edge_in_scope(&e.kind))
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        let mut actual_weights: BTreeMap<(String, String), u32> = BTreeMap::new();
        for e in resp.edges.iter().filter(|e| edge_in_scope(&e.kind)) {
            *actual_weights
                .entry((e.from.clone(), e.to.clone()))
                .or_insert(0) += e.weight;
        }

        let tp = expected_edges.intersection(&actual).count() as f64;
        let fp = actual.difference(&expected_edges).count() as f64;
        let fn_count = expected_edges.difference(&actual).count() as f64;

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
        // F2 — recall priority on edge-set. Missing a real architectural edge
        // (false negative) is worse than over-reporting one (false positive)
        // when the bench reader is checking module structure soundness.
        let f2 = if 4.0 * precision + recall == 0.0 {
            0.0
        } else {
            5.0 * precision * recall / (4.0 * precision + recall)
        };

        // OpenAI/Codex review fix: implement Spearman rank correlation
        // per AS-019 primary metric (F1 fallback was the cycle B
        // workaround). Spearman measures whether the two sides agree
        // on the IMPORTANCE ORDERING of edges, not just whether they
        // agree on the binary "this edge exists" question. F1=1.0 with
        // tautological edge sets can still have spearman < 1.0 if rank
        // orders disagree — that's the engine-quality signal.
        let spearman = compute_spearman_on_shared_edges(
            &expected_edges,
            &actual,
            &expected_weights,
            &actual_weights,
        );

        // Primary metric matches the comparison type: Rust dependency-edge GT
        // is unweighted set membership → F1 on edge-pairs (AS-019). Python
        // import GT keeps Spearman rank-correlation on weights (unchanged).
        let primary = if kind_agnostic {
            f1
        } else {
            spearman.unwrap_or(0.0)
        };

        // When the GT rule produced no expected edges (common on non-Python
        // fixtures whose imports `Ha-import-edge` cannot resolve), there is
        // nothing to score against — Spearman is undefined and the 0.0
        // primary is a degenerate artifact, not an engine failure. Such rows
        // are SKIPPED. Otherwise apply the AS-020 tautology / pass / fail
        // logic. See `decide_spec_status`.
        let has_gt = !expected_edges.is_empty();
        let spec_status = decide_spec_status(f1, primary, has_gt);

        let mut secondary = BTreeMap::new();
        secondary.insert("edge_f1".to_string(), f1);
        secondary.insert("edge_f2".to_string(), f2);
        secondary.insert("edge_precision".to_string(), precision);
        secondary.insert("edge_recall".to_string(), recall);
        secondary.insert(
            "spearman_defined".to_string(),
            if spearman.is_some() { 1.0 } else { 0.0 },
        );
        secondary.insert(
            "expected_edge_count".to_string(),
            expected_edges.len() as f64,
        );
        secondary.insert("actual_edge_count".to_string(), actual.len() as f64);
        secondary.insert(
            "shared_edge_count".to_string(),
            expected_edges.intersection(&actual).count() as f64,
        );
        secondary.insert("true_positives".to_string(), tp);
        secondary.insert("false_positives".to_string(), fp);
        secondary.insert("false_negatives".to_string(), fn_count);

        rows.push(M3LeaderboardRow {
            retriever: retriever_name.clone(),
            fixture: opts.fixture_name.clone(),
            uc: "architecture".to_string(),
            score: primary,
            secondary_metrics: secondary,
            spec_status,
            spec_target: ARCHITECTURE_SPEC_TARGET,
            p95_latency_ms: latency_ms,
        });
    }
    Ok(rows)
}

/// Spearman rank correlation between expected and actual edge weights,
/// computed only on edges shared by both sides (intersection of edge
/// sets). Returns `None` when sample is too small (n < 3) or when one
/// side has all-tied ranks (variance = 0 → undefined correlation).
///
/// Algorithm: standard Spearman via Pearson on rank vectors. Ties get
/// average rank (e.g. 3 values tied for ranks 4-6 all get rank 5.0).
fn compute_spearman_on_shared_edges(
    expected_edges: &BTreeSet<(String, String)>,
    actual_edges: &BTreeSet<(String, String)>,
    expected_weights: &BTreeMap<(String, String), u32>,
    actual_weights: &BTreeMap<(String, String), u32>,
) -> Option<f64> {
    let shared: Vec<(String, String)> =
        expected_edges.intersection(actual_edges).cloned().collect();
    if shared.len() < 3 {
        return None;
    }

    let exp_vals: Vec<f64> = shared
        .iter()
        .map(|k| expected_weights.get(k).copied().unwrap_or(0) as f64)
        .collect();
    let act_vals: Vec<f64> = shared
        .iter()
        .map(|k| actual_weights.get(k).copied().unwrap_or(0) as f64)
        .collect();

    let exp_ranks = average_ranks(&exp_vals);
    let act_ranks = average_ranks(&act_vals);

    // Pearson correlation on the rank vectors = Spearman.
    let n = exp_ranks.len() as f64;
    let mean_x: f64 = exp_ranks.iter().sum::<f64>() / n;
    let mean_y: f64 = act_ranks.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for (x, y) in exp_ranks.iter().zip(act_ranks.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    if var_x < 1e-12 || var_y < 1e-12 {
        return None; // all-tied on one side → undefined
    }
    Some(cov / (var_x * var_y).sqrt())
}

/// Convert raw values to ranks with ties resolved via average-rank.
/// Sort ascending, ties at positions p..q all get rank (p + q + 1) / 2.
fn average_ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        values[a]
            .partial_cmp(&values[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut ranks = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (values[idx[j + 1]] - values[idx[i]]).abs() < 1e-12 {
            j += 1;
        }
        // positions i..=j are ties; assign average of (i+1)..(j+1) (1-based)
        let avg = ((i + 1) as f64 + (j + 1) as f64) / 2.0;
        for k in i..=j {
            ranks[idx[k]] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Decide the spec status for one `architecture` row.
///
/// `has_gt` = the GT rule produced at least one expected edge. When it did
/// NOT, the gate has nothing to measure against — Spearman is undefined and
/// the `0.0` primary score is a degenerate artifact, NOT an engine failure.
/// Such rows are `Skipped` (insufficient ground truth) so they neither count
/// as a hard fail nor masquerade as a pass.
///
/// When GT *does* exist, the original AS-020 logic stands: a near-perfect
/// match on both F1 and Spearman is TAUTOLOGY-SUSPECT; clearing the target is
/// PASS; anything else is a real FAIL (including a genuine low-recall miss
/// where the engine found GT edges but ranked/matched them poorly).
fn decide_spec_status(f1: f64, primary: f64, has_gt: bool) -> SpecStatus {
    if !has_gt {
        return SpecStatus::Skipped;
    }
    if f1 >= 0.95 && primary >= 0.95 {
        SpecStatus::Tautological
    } else if primary >= ARCHITECTURE_SPEC_TARGET {
        SpecStatus::Pass
    } else {
        SpecStatus::Fail
    }
}

fn deferred_row(retriever: &str, opts: &ScoreOpts) -> M3LeaderboardRow {
    let mut row = M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "architecture".to_string(),
        score: 0.0,
        secondary_metrics: BTreeMap::new(),
        spec_status: SpecStatus::Deferred,
        spec_target: ARCHITECTURE_SPEC_TARGET,
        p95_latency_ms: 0,
    };
    row.secondary_metrics
        .insert("note_competitor_adapter_pending".to_string(), 0.0);
    row
}

#[cfg(test)]
mod unit {
    use super::*;

    // Regression: M3 architecture FAIL 0.000 on empty-GT fixtures —
    // m3_architecture.rs decide_spec_status reported Fail when the GT rule
    // produced zero expected edges (non-Python fixtures). Spearman is
    // undefined there; the degenerate 0.0 is not an engine failure.
    #[test]
    fn empty_gt_is_skipped_not_failed() {
        // preact/kotlinx shape: no GT edges → primary defaults to 0.0, and
        // F1 is spuriously 1.0 (precision=recall=1 on the empty set).
        assert_eq!(
            decide_spec_status(1.0, 0.0, false),
            SpecStatus::Skipped,
            "empty GT must SKIP (insufficient ground truth), never FAIL"
        );
    }

    #[test]
    fn real_miss_with_gt_still_fails() {
        // tokio shape: GT has edges (has_gt = true) but the engine matched
        // none → genuine low-recall miss. Must stay FAIL, not be hidden.
        assert_eq!(
            decide_spec_status(0.0, 0.0, true),
            SpecStatus::Fail,
            "a real miss against existing GT must remain FAIL"
        );
    }

    #[test]
    fn clears_target_with_gt_passes() {
        // django shape: F1 perfect, Spearman 0.917 ≥ 0.6 target.
        assert_eq!(decide_spec_status(1.0, 0.917, true), SpecStatus::Pass);
    }

    #[test]
    fn near_perfect_both_is_tautology_suspect() {
        assert_eq!(
            decide_spec_status(0.97, 0.97, true),
            SpecStatus::Tautological
        );
    }
}
