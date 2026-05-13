//! M2 gate runner (S-004 AS-009 / AS-010).
//!
//! Executes the impact benchmark against the unified ground truth
//! (`benches/uc-impact/ground-truth.json`), producing per-fixture +
//! aggregate leaderboards. 7 retrievers × 100 tasks max.
//!
//! Commit pinning is optional via `RunOpts.pin_commits` — when enabled, each
//! task checks out `base_commit` in the fixture submodule before indexing.
//! When disabled (default for iteration speed), the retriever sees whatever
//! state the submodule is currently in — acceptable for smoke but not for
//! gate numbers.

use crate::m2_ground_truth::{M2GroundTruth, M2Task, Split};
use crate::retriever::Retriever;
use crate::retrievers::{
    Bm25Retriever, CgcRetriever, CmRetriever, CrgRetriever, GaRetriever, RandomRetriever,
    RipgrepRetriever,
};
use crate::score::{impact_score, ImpactScore};
use crate::token_cost::{self, TokenCost};
use crate::BenchError;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const M2_RETRIEVER_NAMES: &[&str] = &[
    "ga",
    "codegraphcontext",
    "codebase-memory",
    "code-review-graph",
    "bm25",
    "ripgrep",
    "random",
    // gitnexus: no in-repo adapter yet; add when `crates/ga-bench/src/retrievers/gitnexus.rs` lands.
    // Subprocess retrievers (cgc/cm/crg) graceful-disable when the binary
    // isn't on $PATH — score 0 on all tasks rather than crash the run.
];

pub struct RunOpts {
    pub gt_path: PathBuf,
    pub fixtures_root: PathBuf,
    pub cache_root: PathBuf,
    pub split: Option<Split>,
    pub retrievers: Vec<String>,
    pub pin_commits: bool,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskScore {
    pub task_id: String,
    pub repo: String,
    pub lang: String,
    pub score: ImpactScore,
    pub token_cost: TokenCost,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrieverEntry {
    pub retriever: String,
    pub tasks: Vec<TaskScore>,
    /// Aggregate mean across tasks (simple mean — per-repo breakdown
    /// surfaced separately).
    pub mean_composite: f64,
    pub mean_test_recall: f64,
    pub mean_completeness: f64,
    pub mean_depth_f1: f64,
    pub mean_precision: f64,
    pub p95_latency_ms: u64,
    pub pass_rate: f64, // fraction with composite ≥ 0.80
    // Supplementary (not in composite):
    pub mean_blast_radius_coverage: f64,
    pub mean_adjusted_precision: f64,
    // Token-cost dims (efficiency, decoupled from composite gate).
    //
    // CONDITIONAL means: averaged ONLY over tasks where the retriever
    // reached the threshold. Mixing failures into the mean lets a retriever
    // that returns *fewer* files look cheaper just because its "failure
    // cost = total_returned" is smaller — punishing retrievers that try
    // harder. Use the conditional means for cross-retriever comparison;
    // pair them with `pct_reached_X` so the story stays honest.
    pub mean_tokens_to_50_when_reached: f64,
    pub mean_tokens_to_100_when_reached: f64,
    pub pct_reached_50: f64,
    pub pct_reached_100: f64,
    pub mean_files_returned: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoGroup {
    pub repo: String,
    pub lang: String,
    pub task_count: usize,
    /// retriever name → mean composite on this repo's tasks
    pub per_retriever: BTreeMap<String, RetrieverEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct M2Report {
    pub split: String,
    pub total_tasks: usize,
    pub per_repo: Vec<RepoGroup>,
    pub aggregate: Vec<RetrieverEntry>,
    pub gt_source: String,
    pub spec: String,
}

fn build_retriever(name: &str, cache_root: &Path) -> Option<Box<dyn Retriever>> {
    let r: Box<dyn Retriever> = match name {
        "ga" => Box::new(GaRetriever::new(cache_root.join("ga"))),
        "bm25" => Box::new(Bm25Retriever::new()),
        "random" => Box::new(RandomRetriever::new()),
        "ripgrep" => Box::new(RipgrepRetriever::new()),
        "codegraphcontext" | "cgc" => Box::new(CgcRetriever::new()),
        "codebase-memory" | "cm" => Box::new(CmRetriever::new()),
        "code-review-graph" | "crg" => Box::new(CrgRetriever::new()),
        _ => return None,
    };
    Some(r)
}

use crate::git_pin::{git_checkout, git_head};

fn build_query_json(task: &M2Task) -> serde_json::Value {
    // M2 is a per-symbol bench: input = (seed_symbol, seed_file). Do NOT
    // project `task.source_files` into the query — `source_files ==
    // expected_files` by construction (extract-seeds.ts:86,1128), so any
    // retriever fed `source_files` as input recalls 100% of expected_files
    // by tautology. Earlier "BENCH-FAIR" attempt to surface commit-level
    // input for CRG was reverted 2026-05-04 after the tautology was
    // detected. Per-commit retrievers (CRG-style) are not natively
    // measurable on per-symbol GT — see methodology.md §Fairness audit.
    serde_json::json!({
        "symbol": task.seed_symbol,
        "file": task.seed_file,
    })
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`. This is the SHIM call site (line ~210
// `partition(|p| !is_test_path(p))`) for non-native retrievers — the
// previous stale local copy was the §4.2.5 smoking-gun bias source
// (artificially zeroed competitor Test Recall on Java/Kotlin fixtures).
use ga_query::common::is_test_path;

/// Execute one retriever across all its tasks. Uses query_impact when the
/// retriever implements it natively; otherwise synthesizes from query() by
/// splitting files into source vs test via path pattern.
fn run_retriever_on_tasks(
    retriever: &mut dyn Retriever,
    fixture_dir: &Path,
    tasks: &[&M2Task],
    pin_commits: bool,
) -> Vec<TaskScore> {
    let mut out = Vec::with_capacity(tasks.len());
    let original_head = if pin_commits {
        git_head(fixture_dir).ok()
    } else {
        None
    };

    let mut needs_setup = true;

    for task in tasks {
        // Commit pin — checkout before setup if enabled
        if pin_commits {
            if git_checkout(fixture_dir, &task.base_commit).is_err() {
                // Skip task on checkout failure but record zero score
                out.push(TaskScore {
                    task_id: task.task_id.clone(),
                    repo: task.repo.clone(),
                    lang: task.lang.clone(),
                    score: ImpactScore::default(),
                    token_cost: TokenCost::default(),
                    latency_ms: 0,
                });
                continue;
            }
            needs_setup = true; // state changed, re-setup
        }

        if needs_setup {
            let _ = retriever.setup(fixture_dir);
            needs_setup = false;
        }

        let start = Instant::now();
        let q = build_query_json(task);

        let (files, tests, actual_depth) = match retriever.query_impact(&q) {
            // EXP-DEPTH-BUG-FIX 2026-04-24: was `ia.max_depth` (configured
            // BFS cap) — meant `ia.transitive_completeness` (actual depth
            // BFS reached) per Tools-C24 "BFS reach coverage" semantic.
            // Using the cap let retrievers with hardcoded max_depth (CRG
            // hardcodes 3, CM hardcodes 5) score depth_F1=1.0 trivially
            // on all GT tasks with expected_depth <= hardcoded cap.
            Some(Ok(ia)) => (ia.files, ia.tests, Some(ia.transitive_completeness)),
            Some(Err(_)) | None => {
                // Fallback: plain query → split into files vs tests
                let hits = retriever.query("impact", &q).unwrap_or_default();
                let (f, t): (Vec<_>, Vec<_>) = hits.into_iter().partition(|p| !is_test_path(p));
                (f, t, None)
            }
        };
        let latency = start.elapsed().as_millis() as u64;

        let score = impact_score(
            &files,
            &tests,
            actual_depth,
            &task.expected_files,
            &task.expected_tests,
            task.max_expected_depth,
            &task.should_touch_files,
        );
        let token_cost = token_cost::compute(&files, &task.expected_files, fixture_dir);

        out.push(TaskScore {
            task_id: task.task_id.clone(),
            repo: task.repo.clone(),
            lang: task.lang.clone(),
            score,
            token_cost,
            latency_ms: latency,
        });
    }

    retriever.teardown();

    // Restore HEAD
    if pin_commits {
        if let Some(head) = original_head {
            let _ = git_checkout(fixture_dir, &head);
        }
    }
    out
}

fn aggregate_entry(retriever: &str, tasks: Vec<TaskScore>) -> RetrieverEntry {
    let n = tasks.len() as f64;
    if tasks.is_empty() {
        return RetrieverEntry {
            retriever: retriever.to_string(),
            tasks,
            mean_composite: 0.0,
            mean_test_recall: 0.0,
            mean_completeness: 0.0,
            mean_depth_f1: 0.0,
            mean_precision: 0.0,
            p95_latency_ms: 0,
            pass_rate: 0.0,
            mean_blast_radius_coverage: 0.0,
            mean_adjusted_precision: 0.0,
            mean_tokens_to_50_when_reached: 0.0,
            mean_tokens_to_100_when_reached: 0.0,
            pct_reached_50: 0.0,
            pct_reached_100: 0.0,
            mean_files_returned: 0.0,
        };
    }
    let sum_c: f64 = tasks.iter().map(|t| t.score.composite).sum();
    let sum_tr: f64 = tasks.iter().map(|t| t.score.test_recall).sum();
    let sum_co: f64 = tasks.iter().map(|t| t.score.completeness).sum();
    let sum_df: f64 = tasks.iter().map(|t| t.score.depth_f1).sum();
    let sum_p: f64 = tasks.iter().map(|t| t.score.precision).sum();
    let sum_brc: f64 = tasks.iter().map(|t| t.score.blast_radius_coverage).sum();
    let sum_ap: f64 = tasks.iter().map(|t| t.score.adjusted_precision).sum();
    // Conditional means: only tasks where threshold was actually reached.
    // Otherwise a retriever that returns fewer files looks "cheaper" simply
    // because its failure-budget is smaller, which is backwards.
    let (sum_t50_ok, n_t50_ok): (f64, u32) = tasks
        .iter()
        .filter(|t| t.token_cost.achieved_50)
        .fold((0.0, 0), |(s, c), t| {
            (s + t.token_cost.tokens_to_50 as f64, c + 1)
        });
    let (sum_t100_ok, n_t100_ok): (f64, u32) = tasks
        .iter()
        .filter(|t| t.token_cost.achieved_100)
        .fold((0.0, 0), |(s, c), t| {
            (s + t.token_cost.tokens_to_100 as f64, c + 1)
        });
    let reached_50 = n_t50_ok as f64;
    let reached_100 = n_t100_ok as f64;
    let sum_fr: f64 = tasks.iter().map(|t| t.token_cost.files_returned as f64).sum();
    let mut lats: Vec<u64> = tasks.iter().map(|t| t.latency_ms).collect();
    lats.sort_unstable();
    let idx = ((lats.len() as f64) * 0.95).ceil() as usize;
    let p95 = lats[idx.saturating_sub(1).min(lats.len() - 1)];
    let passed = tasks.iter().filter(|t| t.score.composite >= 0.80).count() as f64;

    RetrieverEntry {
        retriever: retriever.to_string(),
        mean_composite: sum_c / n,
        mean_test_recall: sum_tr / n,
        mean_completeness: sum_co / n,
        mean_depth_f1: sum_df / n,
        mean_precision: sum_p / n,
        p95_latency_ms: p95,
        pass_rate: passed / n,
        mean_blast_radius_coverage: sum_brc / n,
        mean_adjusted_precision: sum_ap / n,
        mean_tokens_to_50_when_reached: if n_t50_ok > 0 {
            sum_t50_ok / n_t50_ok as f64
        } else {
            0.0
        },
        mean_tokens_to_100_when_reached: if n_t100_ok > 0 {
            sum_t100_ok / n_t100_ok as f64
        } else {
            0.0
        },
        pct_reached_50: reached_50 / n,
        pct_reached_100: reached_100 / n,
        mean_files_returned: sum_fr / n,
        tasks,
    }
}

pub fn run(opts: RunOpts) -> Result<M2Report, BenchError> {
    let gt = M2GroundTruth::load(&opts.gt_path)?;
    let split_label = opts
        .split
        .map(|s| match s {
            Split::Dev => "dev",
            Split::Test => "test",
        })
        .unwrap_or("all");

    let filtered = gt.filter_split(opts.split);
    let total = filtered.len();
    println!(
        "M2 bench: {} tasks (split={}), {} retrievers, pin_commits={}",
        total,
        split_label,
        opts.retrievers.len(),
        opts.pin_commits,
    );

    let by_repo = M2GroundTruth::group_by_repo(&filtered);

    // Full task-score matrix, used for aggregate + per-repo reports
    // retriever → all TaskScores across all repos
    let mut retriever_all: BTreeMap<String, Vec<TaskScore>> = BTreeMap::new();
    let mut repos_out: Vec<RepoGroup> = Vec::new();

    let mut repo_names: Vec<&String> = by_repo.keys().collect();
    repo_names.sort();

    for repo in repo_names {
        let tasks = &by_repo[repo];
        let canonical = opts.fixtures_root.join(repo);
        if !canonical.exists() {
            eprintln!(
                "  [{repo}] SKIP: fixture not checked out at {}",
                canonical.display()
            );
            continue;
        }
        // Per-gate scratch ONLY when canonical is a real git repo (i.e. we
        // can/will checkout per-task `base_commit`). Synthetic fixtures
        // without `.git` skip scratch and use canonical directly — pin will
        // also skip in run_retriever_on_tasks. Production fixture path:
        // clone scratch under <cache_root>/fixtures-m2/<repo>; M3 has its
        // own fixtures-m3/ → no cross-gate fixture pollution.
        let fixture_dir = if canonical.join(".git").exists() {
            match crate::fixture_workspace::ensure_gate_scratch("m2", &canonical, &opts.cache_root)
            {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("  [{repo}] SKIP: scratch setup failed: {e}");
                    continue;
                }
            }
        } else {
            canonical.clone()
        };

        // v1.2-php S-002 AS-020 — assert workspace is clean before mining/scoring.
        // Catches the project_m3_submodule_drift failure mode where prior runs
        // left the scratch in a dirty state. Loud-skip the fixture rather than
        // silently producing biased numbers.
        if fixture_dir.join(".git").exists() {
            if let Err(e) = crate::fixture_workspace::assert_workspace_clean(&fixture_dir) {
                eprintln!("  [{repo}] SKIP: FixtureCorrupted precondition failed: {e}");
                continue;
            }
        }
        let lang = tasks[0].lang.clone();
        println!(
            "\n[{repo}] {} tasks ({lang}) — scratch={}",
            tasks.len(),
            fixture_dir.display()
        );

        let mut per_retriever: BTreeMap<String, RetrieverEntry> = BTreeMap::new();
        for name in &opts.retrievers {
            let Some(mut retriever) = build_retriever(name, &opts.cache_root) else {
                eprintln!("  [{name}] unknown retriever, skipping");
                continue;
            };
            let rt_start = Instant::now();
            let scores =
                run_retriever_on_tasks(retriever.as_mut(), &fixture_dir, tasks, opts.pin_commits);
            println!(
                "  [{name}] {} tasks in {:.1}s | mean_composite={:.3}",
                scores.len(),
                rt_start.elapsed().as_secs_f64(),
                scores.iter().map(|t| t.score.composite).sum::<f64>() / scores.len().max(1) as f64,
            );
            let entry = aggregate_entry(name, scores);
            // Add task scores to global retriever bucket
            retriever_all
                .entry(name.clone())
                .or_default()
                .extend(entry.tasks.clone());
            per_retriever.insert(name.clone(), entry);
        }

        repos_out.push(RepoGroup {
            repo: repo.clone(),
            lang,
            task_count: tasks.len(),
            per_retriever,
        });
    }

    let aggregate: Vec<RetrieverEntry> = retriever_all
        .into_iter()
        .map(|(name, tasks)| aggregate_entry(&name, tasks))
        .collect();

    Ok(M2Report {
        split: split_label.to_string(),
        total_tasks: total,
        per_repo: repos_out,
        aggregate,
        gt_source: gt.source.clone(),
        spec: gt.spec.clone(),
    })
}
