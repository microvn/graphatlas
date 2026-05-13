//! `ga_hubs` UC scoring loop for the M3 gate.
//!
//! GT comes from [`crate::gt_gen::hh_gitmine`] — a 12-month git-log file
//! churn ranking, projected against the engine's indexed file set. Engine
//! output is `ga_hubs(top_n=GT_TOP_N)` → file rank by best symbol score
//! per file. Score = Spearman rank correlation on the intersection of
//! the two file sets. Spec target: ρ ≥ 0.7.

use crate::gt_gen::hh_gitmine::HhGitmine;
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, ScoreOpts, SpecStatus};
use crate::BenchError;
use std::collections::{BTreeMap, BTreeSet};

/// Spec target — Spearman ≥ 0.7 (locked with user 2026-05-03).
pub const HUBS_SPEC_TARGET: f64 = 0.7;

/// Match the rule's emit cap so engine + GT have the same headroom.
const TOP_N: u32 = 50;

pub fn score_hubs(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let rule = HhGitmine::default();
    // The rule needs a Store for its scan() signature but doesn't actually
    // use it (file churn is mined directly from git). Open a probe store
    // only because the trait requires one.
    let probe_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&probe_store, &opts.fixture_dir)?;

    if tasks.is_empty() {
        return Ok(Vec::new());
    }
    // Hh-gitmine emits a single per-fixture task — its `expected` is the
    // ranked GT file list (index 0 = most-churned).
    let gt_rank: Vec<String> = tasks[0].expected.clone();
    if gt_rank.is_empty() {
        return Ok(Vec::new());
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
        let resp = ga_query::hubs::hubs(
            &store,
            &ga_query::hubs::HubsRequest {
                top_n: TOP_N,
                symbol: None,
                file: None,
                edge_types: ga_query::hubs::HubsEdgeTypes::Default,
            },
        )
        .map_err(|e| BenchError::Query(format!("hubs: {e}")))?;
        let latency_ms = t0.elapsed().as_millis() as u64;

        // Project symbols → file rank via best (highest) total_degree per
        // file. Position in the resulting Vec is the engine's file rank.
        let mut best_per_file: BTreeMap<String, u32> = BTreeMap::new();
        for h in &resp.hubs {
            let cur = best_per_file.entry(h.file.clone()).or_insert(0);
            if h.total_degree > *cur {
                *cur = h.total_degree;
            }
        }
        let mut engine_rank: Vec<(String, u32)> = best_per_file.into_iter().collect();
        engine_rank.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let engine_files: Vec<String> = engine_rank.into_iter().map(|(f, _)| f).collect();

        // Compute Spearman on the intersection of the two file sets so we
        // don't penalise the engine for files git-mined GT lists outside
        // the indexer's universe (e.g. submodules, generated paths).
        let gt_set: BTreeSet<&String> = gt_rank.iter().collect();
        let engine_set: BTreeSet<&String> = engine_files.iter().collect();
        let common: BTreeSet<&String> = gt_set.intersection(&engine_set).copied().collect();

        let (spearman, common_count) = if common.len() < 3 {
            // Spearman undefined for n < 2; even n = 2 is degenerate. Mark
            // as 0.0 so a tiny / fresh fixture doesn't register as PASS by
            // accident.
            (0.0, common.len())
        } else {
            let gt_pos: BTreeMap<&String, usize> = gt_rank
                .iter()
                .enumerate()
                .filter(|(_, f)| common.contains(f))
                .map(|(i, f)| (f, i))
                .collect();
            let engine_pos: BTreeMap<&String, usize> = engine_files
                .iter()
                .enumerate()
                .filter(|(_, f)| common.contains(f))
                .map(|(i, f)| (f, i))
                .collect();
            let pairs: Vec<(usize, usize)> = common
                .iter()
                .filter_map(|f| {
                    let g = *gt_pos.get(f)?;
                    let e = *engine_pos.get(f)?;
                    Some((g, e))
                })
                .collect();
            (spearman_rho(&pairs), common.len())
        };

        let mut secondary = BTreeMap::new();
        secondary.insert("gt_size".to_string(), gt_rank.len() as f64);
        secondary.insert("engine_size".to_string(), engine_files.len() as f64);
        secondary.insert("common_files".to_string(), common_count as f64);
        secondary.insert("total_hubs_with_edges".to_string(), resp.hubs.len() as f64);

        let spec_status = if spearman >= HUBS_SPEC_TARGET {
            SpecStatus::Pass
        } else {
            SpecStatus::Fail
        };

        rows.push(M3LeaderboardRow {
            retriever: retriever_name.clone(),
            fixture: opts.fixture_name.clone(),
            uc: "hubs".to_string(),
            score: spearman,
            secondary_metrics: secondary,
            spec_status,
            spec_target: HUBS_SPEC_TARGET,
            p95_latency_ms: latency_ms,
        });
    }

    Ok(rows)
}

/// Spearman rank correlation on integer-rank pairs. Pairs are
/// `(rank_in_gt, rank_in_engine)`; returns ρ ∈ [-1, 1].
fn spearman_rho(pairs: &[(usize, usize)]) -> f64 {
    let n = pairs.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    // Pearson on the two rank vectors == Spearman when ranks are the same
    // monotone transform on each side (no ties present in pure 0..k-1
    // ranks); we sort each side independently to translate "position in
    // common-file ordering" to dense ranks.
    let xs: Vec<f64> = ranks(pairs.iter().map(|p| p.0 as f64).collect::<Vec<_>>());
    let ys: Vec<f64> = ranks(pairs.iter().map(|p| p.1 as f64).collect::<Vec<_>>());

    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut dx2 = 0.0;
    let mut dy2 = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        num += dx * dy;
        dx2 += dx * dx;
        dy2 += dy * dy;
    }
    let denom = (dx2 * dy2).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        num / denom
    }
}

/// Dense ranking: smallest value gets rank 1.0, ties share averaged rank.
fn ranks(values: Vec<f64>) -> Vec<f64> {
    let n = values.len();
    let mut indexed: Vec<(usize, f64)> = values.iter().enumerate().map(|(i, &v)| (i, v)).collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && indexed[j + 1].1 == indexed[i].1 {
            j += 1;
        }
        // Average rank for the tie group, 1-based.
        let rank_avg = ((i + 1) as f64 + (j + 1) as f64) / 2.0;
        for k in i..=j {
            out[indexed[k].0] = rank_avg;
        }
        i = j + 1;
    }
    out
}

fn deferred_row(retriever: &str, opts: &ScoreOpts) -> M3LeaderboardRow {
    let mut row = M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "hubs".to_string(),
        score: 0.0,
        secondary_metrics: BTreeMap::new(),
        spec_status: SpecStatus::Deferred,
        spec_target: HUBS_SPEC_TARGET,
        p95_latency_ms: 0,
    };
    row.secondary_metrics
        .insert("note_competitor_adapter_pending".to_string(), 0.0);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spearman_perfect_positive_is_one() {
        let pairs = vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)];
        let rho = spearman_rho(&pairs);
        assert!((rho - 1.0).abs() < 1e-9, "got {rho}");
    }

    #[test]
    fn spearman_perfect_negative_is_minus_one() {
        let pairs = vec![(0, 4), (1, 3), (2, 2), (3, 1), (4, 0)];
        let rho = spearman_rho(&pairs);
        assert!((rho + 1.0).abs() < 1e-9, "got {rho}");
    }

    #[test]
    fn spearman_uncorrelated_is_near_zero() {
        // Permutation chosen so neither monotone — expect |rho| << 1.
        let pairs = vec![(0, 2), (1, 4), (2, 1), (3, 3), (4, 0)];
        let rho = spearman_rho(&pairs);
        assert!(rho.abs() < 0.6, "expected near-zero correlation; got {rho}");
    }

    #[test]
    fn ranks_handles_ties() {
        let r = ranks(vec![10.0, 10.0, 20.0]);
        assert_eq!(r, vec![1.5, 1.5, 3.0]);
    }
}
