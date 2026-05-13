//! Minimal-context UC scoring loop for the M3 gate.
//!
//! Migrated 2026-04-28 from `Hmc-budget` (archived tasks-v6 LLM dataset)
//! to `Hmc-gitmine` (M1+M2 git-mining ground-truth.json). See
//! `archive/README.md` for migration history.

use crate::git_pin::{git_checkout, git_head};
use crate::gt_gen::hmc_gitmine::{HmcGitmine, Split};
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, SpecStatus};
use crate::m3_score::{
    file_recall, recall_per_1k_tokens, test_recall, truncation_correctness_rate,
};
use crate::BenchError;
use std::collections::BTreeMap;

/// Options bag for the minimal_context scoring loop. Exposed so
/// integration tests can drive synthetic fixtures without going through
/// the CLI (which hard-codes `benches/fixtures/<name>/`).
#[derive(Debug, Clone)]
pub struct ScoreOpts {
    pub fixture_name: String,
    pub fixture_dir: std::path::PathBuf,
    pub cache_root: std::path::PathBuf,
    pub retrievers: Vec<String>,
    /// Optional override for the ground-truth.json path — used by tests
    /// that ship a synthetic dataset under tempdir.
    pub gt_path: Option<std::path::PathBuf>,
    /// Optional split override. `None` = `Split::Test` (gate default).
    pub split: Option<Split>,
}

/// Spec target — same bar as other 4 M3 UCs (architecture/dead_code/risk
/// PASS at 0.857-0.917). User direction 2026-04-28: no double-standard,
/// even though minimal_context is fundamentally narrower than impact.
/// Sub-target → honest signal "static graph not enough for this UC".
pub const MINIMAL_CONTEXT_SPEC_TARGET: f64 = 0.70;
pub const MINIMAL_CONTEXT_BUDGET: u32 = 2000;

pub fn score_minimal_context(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let mut rule = HmcGitmine::for_fixture(&opts.fixture_name);
    if let Some(p) = &opts.gt_path {
        rule = rule.with_gt_path(p.clone());
    }
    if let Some(s) = opts.split {
        rule = rule.with_split(s);
    }
    // The rule reads ground-truth.json off-disk and doesn't actually use
    // the store — but the trait signature requires one. Open a probe store
    // co-located with cache_root so we satisfy the trait without an
    // extra fixture-specific build_index.
    let gt_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&gt_store, &opts.fixture_dir)?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::new();
    for retriever_name in &opts.retrievers {
        if retriever_name != "ga" {
            // Phase 1+2 measures only `ga` against spec target; competitor
            // adapters land Phase 4. Skip with a placeholder DEFERRED row.
            let mut row = M3LeaderboardRow {
                retriever: retriever_name.clone(),
                fixture: opts.fixture_name.clone(),
                uc: "minimal_context".to_string(),
                score: 0.0,
                secondary_metrics: BTreeMap::new(),
                spec_status: SpecStatus::Deferred,
                spec_target: MINIMAL_CONTEXT_SPEC_TARGET,
                p95_latency_ms: 0,
            };
            row.secondary_metrics
                .insert("note_competitor_adapter_pending".to_string(), 0.0);
            rows.push(row);
            continue;
        }
        rows.push(score_ga(opts, &tasks)?);
    }
    Ok(rows)
}

struct PerTask {
    file_recall: f64,
    file_precision: f64,
    test_recall: f64,
    eff: f64,
    truncation: (bool, bool),
    latency_ms: u64,
}

fn score_ga(
    opts: &ScoreOpts,
    tasks: &[crate::gt_gen::GeneratedTask],
) -> Result<M3LeaderboardRow, BenchError> {
    // Save fixture HEAD so we can restore after pin iteration. If git_head
    // fails (e.g. fixture not a git repo — synthetic test tempdir), we won't
    // pin — record the condition but proceed so the bench produces SOME
    // signal.
    let original_head = git_head(&opts.fixture_dir).ok();
    let pin_enabled = original_head.is_some();

    // Per-gate scratch ONLY when we'll mutate (pin_enabled). Synthetic
    // non-git fixtures use the source dir directly — same as pre-Phase-5
    // behaviour. Production fixture (real submodule) → clone scratch under
    // <cache_root>/fixtures-m3/<repo> so canonical stays immutable. Cross-
    // gate fixture pollution impossible by construction.
    let fixture_dir = if pin_enabled {
        match crate::fixture_workspace::ensure_gate_scratch(
            "m3",
            &opts.fixture_dir,
            &opts.cache_root,
        ) {
            Ok(p) => p,
            Err(e) => {
                return Err(BenchError::Other(anyhow::anyhow!(
                    "m3 scratch setup failed: {e}"
                )));
            }
        }
    } else {
        opts.fixture_dir.clone()
    };

    let mut per_task: Vec<PerTask> = Vec::with_capacity(tasks.len());
    let mut seed_symbol_not_found_count: u32 = 0;
    // 2026-04-28: distinguish "hint miss" from "no symbol anywhere" so the
    // leaderboard reader can tell stale GT from indexer extraction gap.
    // Both paths score 0 — counter is a diagnostic only.
    let mut seed_symbol_not_found_at_hinted_file_count: u32 = 0;
    let mut pin_failed_count: u32 = 0;

    for task in tasks {
        let symbol = task
            .query
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let base_commit = task
            .query
            .get("__base_commit")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let seed_file = task
            .query
            .get("__seed_file")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let must_touch_test_files: Vec<String> = task
            .query
            .get("__expected_tests")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Pin per-task base_commit (M2 policy). Score 0 + count if checkout
        // fails — distinct counter from "engine couldn't resolve symbol"
        // so leaderboard reader can tell "fixture stale" from "engine weak".
        if pin_enabled
            && !base_commit.is_empty()
            && git_checkout(&fixture_dir, base_commit).is_err()
        {
            pin_failed_count += 1;
            per_task.push(PerTask {
                file_recall: 0.0,
                file_precision: 0.0,
                test_recall: 0.0,
                eff: 0.0,
                truncation: (false, false),
                latency_ms: 0,
            });
            continue;
        }

        // Re-build index at the pinned commit. Each task gets its own
        // ephemeral cache so prior tasks don't pollute.
        let cache = opts.cache_root.join("ga").join(&task.task_id);
        let _ = std::fs::remove_dir_all(&cache);
        let store = match ga_index::Store::open_with_root(&cache, &fixture_dir) {
            Ok(s) => s,
            Err(e) => return Err(BenchError::Query(format!("open store: {e}"))),
        };
        if let Err(e) = ga_query::indexer::build_index(&store, &fixture_dir) {
            return Err(BenchError::Other(anyhow::anyhow!("build_index: {e}")));
        }

        if symbol.is_empty() {
            // GT row had no seed_symbol — skip with a 0 score so the row
            // count stays consistent. (Shouldn't happen with current GT.)
            per_task.push(PerTask {
                file_recall: 0.0,
                file_precision: 0.0,
                test_recall: 0.0,
                eff: 0.0,
                truncation: (false, false),
                latency_ms: 0,
            });
            continue;
        }

        let req = match seed_file {
            Some(f) => ga_query::minimal_context::MinimalContextRequest::for_symbol_in_file(
                symbol,
                f,
                MINIMAL_CONTEXT_BUDGET,
            ),
            None => ga_query::minimal_context::MinimalContextRequest::for_symbol(
                symbol,
                MINIMAL_CONTEXT_BUDGET,
            ),
        };

        let t0 = std::time::Instant::now();
        let resp = match ga_query::minimal_context::minimal_context(&store, &req) {
            Ok(r) => r,
            Err(_) => {
                if seed_file.is_some() {
                    seed_symbol_not_found_at_hinted_file_count += 1;
                } else {
                    seed_symbol_not_found_count += 1;
                }
                per_task.push(PerTask {
                    file_recall: 0.0,
                    file_precision: 0.0,
                    test_recall: 0.0,
                    eff: 0.0,
                    truncation: (false, false),
                    latency_ms: t0.elapsed().as_millis() as u64,
                });
                continue;
            }
        };
        let latency_ms = t0.elapsed().as_millis() as u64;

        let actual_files: Vec<String> = resp
            .symbols
            .iter()
            .map(|s| s.file.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        let fr = file_recall(&actual_files, &task.expected);
        let fp = file_precision(&actual_files, &task.expected);
        let tr = test_recall(&actual_files, &must_touch_test_files);
        let eff = recall_per_1k_tokens(fr, resp.token_estimate);

        per_task.push(PerTask {
            file_recall: fr,
            file_precision: fp,
            test_recall: tr,
            eff,
            truncation: (
                resp.meta.truncated,
                resp.token_estimate > MINIMAL_CONTEXT_BUDGET,
            ),
            latency_ms,
        });
    }

    // Restore fixture HEAD so subsequent benches start from a known state.
    if let Some(orig) = &original_head {
        let _ = git_checkout(&fixture_dir, orig);
    }

    let n = per_task.len() as f64;
    let mean = |sel: fn(&PerTask) -> f64| {
        if n == 0.0 {
            0.0
        } else {
            per_task.iter().map(sel).sum::<f64>() / n
        }
    };
    let mean_file = mean(|p| p.file_recall);
    let mean_prec = mean(|p| p.file_precision);
    let mean_test = mean(|p| p.test_recall);
    let mean_eff = mean(|p| p.eff);
    let trunc =
        truncation_correctness_rate(&per_task.iter().map(|p| p.truncation).collect::<Vec<_>>());

    let mut latencies: Vec<u64> = per_task.iter().map(|p| p.latency_ms).collect();
    latencies.sort_unstable();
    let p95_idx = if latencies.is_empty() {
        0
    } else {
        ((latencies.len() as f64) * 0.95).ceil() as usize - 1
    };
    let p95 = latencies.get(p95_idx).copied().unwrap_or(0);

    let mut secondary = BTreeMap::new();
    secondary.insert("file_precision".to_string(), mean_prec);
    secondary.insert("test_recall".to_string(), mean_test);
    secondary.insert("recall_per_1k_tokens".to_string(), mean_eff);
    secondary.insert("truncation_correctness_rate".to_string(), trunc);
    secondary.insert("task_count".to_string(), n);
    secondary.insert(
        "seed_symbol_not_found_count".to_string(),
        seed_symbol_not_found_count as f64,
    );
    secondary.insert(
        "seed_symbol_not_found_at_hinted_file_count".to_string(),
        seed_symbol_not_found_at_hinted_file_count as f64,
    );
    secondary.insert("pin_failed_count".to_string(), pin_failed_count as f64);
    secondary.insert(
        "pin_enabled".to_string(),
        if pin_enabled { 1.0 } else { 0.0 },
    );

    let spec_status = if mean_file >= MINIMAL_CONTEXT_SPEC_TARGET {
        SpecStatus::Pass
    } else {
        SpecStatus::Fail
    };

    Ok(M3LeaderboardRow {
        retriever: "ga".to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "minimal_context".to_string(),
        score: mean_file,
        secondary_metrics: secondary,
        spec_status,
        spec_target: MINIMAL_CONTEXT_SPEC_TARGET,
        p95_latency_ms: p95,
    })
}

/// |actual ∩ expected| / |actual|. Matching `file_recall` semantics: when
/// the actual set is empty we return 1.0 (vacuous — nothing to be wrong
/// about), so the metric stays in [0,1] without poisoning the mean.
fn file_precision(actual_files: &[String], expected_files: &[String]) -> f64 {
    if actual_files.is_empty() {
        return 1.0;
    }
    let expected: std::collections::BTreeSet<&str> =
        expected_files.iter().map(String::as_str).collect();
    let hits = actual_files
        .iter()
        .filter(|f| expected.contains(f.as_str()))
        .count();
    hits as f64 / actual_files.len() as f64
}
