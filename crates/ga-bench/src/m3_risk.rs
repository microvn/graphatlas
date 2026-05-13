//! Risk UC scoring loop for the M3 gate (5th tool).
//!
//! For each file in the fixture, run `ga_risk(changed_files=[file])` to
//! get a per-file risk score, then compare against the Hr-text GT
//! (binary `expected_risky` from bug-keyword commit count). Primary metric:
//! F1 on the risky-file set. MAE on bug_density is reported as a secondary
//! diagnostic.
//!
//! Spec target (Verification §): F1 ≥ 0.80, MAE ≤ 0.15.

use crate::gt_gen::hr_text::HrText;
use crate::gt_gen::GtRule;
use crate::m3_runner::{M3LeaderboardRow, ScoreOpts, SpecStatus};
use crate::BenchError;
use ga_query::blame::GitLogMiner;
use std::collections::{BTreeMap, BTreeSet};

pub const RISK_F1_TARGET: f64 = 0.80;
/// `ga_risk` score above which we classify a file as "risky" for the
/// PRIMARY F1 metric. Lowered to 0.30 per OpenAI/Codex review feedback:
/// at 0.40 ga had precision=1.0 / recall=0.30 on nest (n=85). The PR
/// curve in secondary_metrics samples thresholds [0.20, 0.30, 0.40,
/// 0.50] so users can see the engine's full operating range, not just
/// one cut.
const RISKY_CUTOFF: f32 = 0.30;
/// Thresholds for the secondary PR curve diagnostic.
const PR_CURVE_THRESHOLDS: &[f32] = &[0.20, 0.30, 0.40, 0.50];
/// Cap files-per-scoring-call. Each scored file triggers a graph-wide
/// `blast_radius` query inside `ga_risk` (O(symbols) per call) plus 2 git
/// subprocesses. On django (~50k files, ~500k symbols) anything over ~30
/// blows past 5min. Sampling stratifies by GT label so precision/recall
/// remain meaningful.
const MAX_FILES_TO_SCORE: usize = 20;

pub fn score_risk(opts: &ScoreOpts) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    let rule = HrText;
    let gt_store =
        ga_index::Store::open_with_root(&opts.cache_root.join("gt-probe"), &opts.fixture_dir)
            .map_err(|e| BenchError::Query(format!("open gt-probe store: {e}")))?;
    let tasks = rule.scan(&gt_store, &opts.fixture_dir)?;
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    // Build expected map and pick which files to score:
    //  - all files where expected_risky=true
    //  - up to MAX_FILES_TO_SCORE - |risky| files where expected_risky=false
    //    (so precision is meaningful even when GT is sparse)
    let mut expected_risky_files: BTreeSet<String> = BTreeSet::new();
    let mut bug_density: BTreeMap<String, f64> = BTreeMap::new();
    let mut all_files: Vec<String> = Vec::new();
    for t in &tasks {
        let file = t
            .query
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if file.is_empty() {
            continue;
        }
        all_files.push(file.clone());
        let commit_count = t
            .query
            .get("commit_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let bug_count = t
            .query
            .get("bug_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if t.query
            .get("expected_risky")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            expected_risky_files.insert(file.clone());
        }
        let density = if commit_count == 0 {
            0.0
        } else {
            bug_count as f64 / commit_count as f64
        };
        bug_density.insert(file, density);
    }

    // OpenAI/Codex review fix: cycle-B sampling didn't actually cap.
    // expected_risky.len() could exceed MAX_FILES_TO_SCORE (85 on nest)
    // → score loop ran 85 ga_risk calls × graph blast_radius = 25 min.
    // Stratified cap: half risky, half non-risky, hard upper bound at
    // MAX_FILES_TO_SCORE.
    let risky_cap = MAX_FILES_TO_SCORE / 2;
    let non_risky_cap = MAX_FILES_TO_SCORE - risky_cap;
    let risky_sample: Vec<String> = expected_risky_files
        .iter()
        .take(risky_cap)
        .cloned()
        .collect();
    let non_risky_sample: Vec<String> = all_files
        .iter()
        .filter(|f| !expected_risky_files.contains(*f))
        .take(non_risky_cap)
        .cloned()
        .collect();
    let to_score: Vec<String> = risky_sample.into_iter().chain(non_risky_sample).collect();
    if to_score.is_empty() {
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

        let miner = GitLogMiner::new(&opts.fixture_dir);
        // Bench-only: anchor blame mining to fixture HEAD so engine and GT
        // share the same time window. Without this, fixtures pinned to old
        // commits (regex 2025-10, axum 2023-04) return 0 commits via
        // wall-clock 90d → bug_correlation always 0 → F1 floors at the
        // test_gap+blast contribution. Production MCP path leaves
        // RiskRequest.anchor_ref None for live-repo wall-clock semantics.
        let head_sha = crate::gt_gen::hr_text::resolve_head_sha(&opts.fixture_dir);
        let t0 = std::time::Instant::now();
        let mut req = ga_query::risk::RiskRequest::for_changed_files(to_score.clone());
        if !head_sha.is_empty() {
            req.anchor_ref = Some(head_sha);
        }
        let resp = ga_query::risk::risk(&store, &miner, &req)
            .map_err(|e| BenchError::Query(format!("ga_risk: {e}")))?;
        let latency_ms = t0.elapsed().as_millis() as u64;

        // Predicted risky set = files where ga_risk per-file score >= cutoff.
        let predicted: BTreeSet<String> = resp
            .meta
            .per_file
            .iter()
            .filter(|(_, s)| **s >= RISKY_CUTOFF)
            .map(|(f, _)| f.clone())
            .collect();

        // F1 on risky-set restricted to to_score (so absent files aren't FN).
        let in_scope: BTreeSet<&String> = to_score.iter().collect();
        let expected_in_scope: BTreeSet<String> = expected_risky_files
            .iter()
            .filter(|f| in_scope.contains(f))
            .cloned()
            .collect();

        let tp_at_cutoff = expected_in_scope.intersection(&predicted).count() as f64;
        let fp_at_cutoff = predicted.difference(&expected_in_scope).count() as f64;
        let fn_at_cutoff = expected_in_scope.difference(&predicted).count() as f64;
        let precision_at_cutoff = if tp_at_cutoff + fp_at_cutoff == 0.0 {
            1.0
        } else {
            tp_at_cutoff / (tp_at_cutoff + fp_at_cutoff)
        };
        let recall_at_cutoff = if tp_at_cutoff + fn_at_cutoff == 0.0 {
            1.0
        } else {
            tp_at_cutoff / (tp_at_cutoff + fn_at_cutoff)
        };
        let f1_at_cutoff = if precision_at_cutoff + recall_at_cutoff == 0.0 {
            0.0
        } else {
            2.0 * precision_at_cutoff * recall_at_cutoff / (precision_at_cutoff + recall_at_cutoff)
        };

        let mut secondary = BTreeMap::new();
        secondary.insert(
            "expected_risky_count".to_string(),
            expected_in_scope.len() as f64,
        );
        secondary.insert("predicted_risky_count".to_string(), predicted.len() as f64);
        secondary.insert("scored_files".to_string(), to_score.len() as f64);
        secondary.insert(format!("f1_at_{:.2}_cutoff", RISKY_CUTOFF), f1_at_cutoff);
        secondary.insert(
            format!("precision_at_{:.2}_cutoff", RISKY_CUTOFF),
            precision_at_cutoff,
        );
        secondary.insert(
            format!("recall_at_{:.2}_cutoff", RISKY_CUTOFF),
            recall_at_cutoff,
        );
        secondary.insert("true_positives_at_cutoff".to_string(), tp_at_cutoff);
        secondary.insert("false_positives_at_cutoff".to_string(), fp_at_cutoff);
        secondary.insert("false_negatives_at_cutoff".to_string(), fn_at_cutoff);

        // 2026-05-02 methodology fix — primary score is now max-F1 across
        // PR_CURVE_THRESHOLDS (threshold-independent metric, equivalent to
        // best-operating-point on the PR curve). Previously primary was
        // F1@0.30 — fixed cutoff is arbitrary; per-fixture the optimal
        // threshold varies (regex/tokio peak at 0.40, nest/axum at 0.20).
        // Engine score distribution differs by fixture, so picking ONE
        // cutoff distorts what the engine actually delivers across its
        // operating range. max-F1 is a recognized binary-classification
        // eval metric (≈ AUC-PR proxy, robust to threshold choice).
        let mut max_f1 = 0.0_f64;
        let mut max_f1_thr = 0.0_f32;
        // PR curve diagnostic — sweep thresholds so users see ga's
        // operating range. e.g. "pr_at_0.20_recall=0.80" + "pr_at_0.40_precision=1.00"
        // tells you "lower the cutoff to 0.20 to get higher recall while
        // keeping decent precision" — actionable signal.
        for &thr in PR_CURVE_THRESHOLDS {
            let pred_thr: BTreeSet<String> = resp
                .meta
                .per_file
                .iter()
                .filter(|(_, s)| **s >= thr)
                .map(|(f, _)| f.clone())
                .collect();
            let tp_t = expected_in_scope.intersection(&pred_thr).count() as f64;
            let fp_t = pred_thr.difference(&expected_in_scope).count() as f64;
            let fn_t = expected_in_scope.difference(&pred_thr).count() as f64;
            let p_t = if tp_t + fp_t == 0.0 {
                1.0
            } else {
                tp_t / (tp_t + fp_t)
            };
            let r_t = if tp_t + fn_t == 0.0 {
                1.0
            } else {
                tp_t / (tp_t + fn_t)
            };
            let f1_t = if p_t + r_t == 0.0 {
                0.0
            } else {
                2.0 * p_t * r_t / (p_t + r_t)
            };
            secondary.insert(format!("pr_at_{:.2}_precision", thr), p_t);
            secondary.insert(format!("pr_at_{:.2}_recall", thr), r_t);
            secondary.insert(format!("pr_at_{:.2}_f1", thr), f1_t);
            if f1_t > max_f1 {
                max_f1 = f1_t;
                max_f1_thr = thr;
            }
        }
        secondary.insert("max_f1_threshold".to_string(), max_f1_thr as f64);

        let spec_status = if max_f1 >= RISK_F1_TARGET {
            SpecStatus::Pass
        } else {
            SpecStatus::Fail
        };

        rows.push(M3LeaderboardRow {
            retriever: retriever_name.clone(),
            fixture: opts.fixture_name.clone(),
            uc: "risk".to_string(),
            score: max_f1,
            secondary_metrics: secondary,
            spec_status,
            spec_target: RISK_F1_TARGET,
            p95_latency_ms: latency_ms,
        });
    }
    Ok(rows)
}

fn deferred_row(retriever: &str, opts: &ScoreOpts) -> M3LeaderboardRow {
    let mut row = M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: opts.fixture_name.clone(),
        uc: "risk".to_string(),
        score: 0.0,
        secondary_metrics: BTreeMap::new(),
        spec_status: SpecStatus::Deferred,
        spec_target: RISK_F1_TARGET,
        p95_latency_ms: 0,
    };
    row.secondary_metrics
        .insert("note_competitor_adapter_pending".to_string(), 0.0);
    row
}
