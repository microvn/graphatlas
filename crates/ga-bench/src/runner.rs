//! Bench runner — validate inputs (AS-002/AS-003), drive one or more
//! retrievers through each GT task, score the actuals, and write the
//! Markdown leaderboard. Keeps retrievers behind the trait so new tools
//! (cgc, cm, crg) slot in without touching this module.

use crate::leaderboard::{write_leaderboard, LeaderEntry, Leaderboard};
use crate::retriever::Retriever;
use crate::retrievers::{CgcRetriever, CmRetriever, CrgRetriever, GaRetriever, RipgrepRetriever};
use crate::score::{f1, mrr, precision, recall};
use crate::{BenchError, M1GroundTruth};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Names recognized by [`build_retrievers`]. Keep aligned with the module
/// set under [`crate::retrievers`].
pub const RETRIEVER_NAMES: &[&str] = &[
    "ga",
    "ripgrep",
    "codegraphcontext",
    "codebase-memory",
    "code-review-graph",
];

/// Resolve a list of retriever names (as from `--retrievers ga,cgc,…`) into
/// boxed trait objects. Preserves caller order so the leaderboard renders
/// in the order the user asked for. Unknown names surface as typed errors
/// with the supported list attached.
pub fn build_retrievers(
    names: &[&str],
    cache_root: PathBuf,
) -> Result<Vec<Box<dyn Retriever>>, BenchError> {
    if names.is_empty() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "no retrievers requested — supported: {}",
            RETRIEVER_NAMES.join(", ")
        )));
    }
    let mut out: Vec<Box<dyn Retriever>> = Vec::with_capacity(names.len());
    for name in names {
        let r: Box<dyn Retriever> = match *name {
            "ga" => Box::new(GaRetriever::new(cache_root.clone())),
            "ripgrep" => Box::new(RipgrepRetriever::new()),
            "codegraphcontext" | "cgc" => Box::new(CgcRetriever::new()),
            "codebase-memory" | "cm" => Box::new(CmRetriever::new()),
            "code-review-graph" | "crg" => Box::new(CrgRetriever::new()),
            other => {
                return Err(BenchError::Other(anyhow::anyhow!(
                    "unknown retriever `{other}` — supported: {}",
                    RETRIEVER_NAMES.join(", ")
                )))
            }
        };
        out.push(r);
    }
    Ok(out)
}

/// AS-002 + AS-003 gate. Returns the loaded ground truth on success so
/// callers can proceed to retriever execution.
pub fn validate_inputs(fixture_dir: &Path, gt_path: &Path) -> Result<M1GroundTruth, BenchError> {
    if !fixture_dir.exists() || !fixture_dir.is_dir() {
        return Err(BenchError::FixtureMissing {
            path: fixture_dir.display().to_string(),
        });
    }
    let mut empty = true;
    for entry in std::fs::read_dir(fixture_dir)? {
        let _ = entry?;
        empty = false;
        break;
    }
    if empty {
        return Err(BenchError::FixtureMissing {
            path: fixture_dir.display().to_string(),
        });
    }
    M1GroundTruth::load(gt_path)
}

/// Conventional on-disk layout: `benches/fixtures/<fixture>/`,
/// `benches/uc-<uc>/<fixture>.json`, `bench-results/<uc>-<fixture>-leaderboard.md`.
pub struct UcLayout {
    pub fixture_dir: PathBuf,
    pub gt_path: PathBuf,
    pub out_md: PathBuf,
}

impl UcLayout {
    pub fn for_uc(repo_root: &Path, uc: &str, fixture: &str) -> Self {
        Self {
            fixture_dir: repo_root.join("benches").join("fixtures").join(fixture),
            gt_path: repo_root
                .join("benches")
                .join(format!("uc-{uc}"))
                .join(format!("{fixture}.json")),
            out_md: repo_root
                .join("bench-results")
                .join(format!("{uc}-{fixture}-leaderboard.md")),
        }
    }
}

pub struct RunOpts {
    pub uc: String,
    pub fixture_dir: PathBuf,
    pub gt_path: PathBuf,
    pub cache_root: PathBuf,
    pub out_md: PathBuf,
}

/// Execute `uc` with the full default retriever set: ga + ripgrep +
/// codegraphcontext + codebase-memory. External retrievers disable
/// gracefully when their tool isn't installed, so this is safe as the CLI
/// default.
pub fn run_uc(opts: RunOpts) -> Result<Leaderboard, BenchError> {
    let retrievers = build_retrievers(RETRIEVER_NAMES, opts.cache_root.clone())?;
    run_uc_with(opts, retrievers)
}

/// Explicit-retrievers variant. Each retriever runs `setup`, every GT task,
/// then `teardown`. An error from `setup` on any retriever is logged and
/// that retriever is skipped — the bench continues with the others so one
/// broken external tool can't nuke the whole run.
pub fn run_uc_with(
    opts: RunOpts,
    mut retrievers: Vec<Box<dyn Retriever>>,
) -> Result<Leaderboard, BenchError> {
    let gt = validate_inputs(&opts.fixture_dir, &opts.gt_path)?;
    if gt.uc != opts.uc {
        return Err(BenchError::GroundTruthMalformed {
            path: opts.gt_path.display().to_string(),
            reason: format!("GT UC `{}` does not match requested `{}`", gt.uc, opts.uc),
        });
    }

    let mut entries = Vec::new();
    for r in retrievers.iter_mut() {
        match run_one_retriever(r.as_mut(), &opts.uc, &opts.fixture_dir, &gt) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!("bench: retriever `{}` failed: {e}", r.name());
                entries.push(LeaderEntry {
                    retriever: r.name().to_string(),
                    f1: 0.0,
                    recall: 0.0,
                    precision: 0.0,
                    mrr: 0.0,
                    p95_latency_ms: 0,
                    pass_rate: 0.0,
                });
            }
        }
        r.teardown();
    }

    let lb = Leaderboard {
        uc: opts.uc.clone(),
        fixture: gt.fixture.clone(),
        hardware: default_hardware(),
        entries,
    };

    if let Some(parent) = opts.out_md.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_leaderboard(&lb, &opts.out_md)?;
    Ok(lb)
}

fn run_one_retriever(
    retriever: &mut dyn Retriever,
    uc: &str,
    fixture_dir: &Path,
    gt: &M1GroundTruth,
) -> Result<LeaderEntry, BenchError> {
    retriever.setup(fixture_dir)?;

    let is_ranked = uc == "symbols"; // AS-007 — MRR-scored
    let mut f1s = Vec::with_capacity(gt.tasks.len());
    let mut precisions = Vec::with_capacity(gt.tasks.len());
    let mut recalls = Vec::with_capacity(gt.tasks.len());
    let mut mrrs = Vec::with_capacity(gt.tasks.len());
    let mut latencies_ms = Vec::with_capacity(gt.tasks.len());
    let mut passed = 0usize;

    for task in &gt.tasks {
        let start = Instant::now();
        let actual = retriever.query(uc, &task.query)?;
        latencies_ms.push(start.elapsed().as_millis() as u64);

        if is_ranked {
            let target = task.expected.first().cloned().unwrap_or_default();
            let act_refs: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
            let m = mrr(&act_refs, &target.as_str());
            mrrs.push(m);
            if m >= 0.5 {
                passed += 1;
            }
        } else {
            let exp: Vec<&str> = task.expected.iter().map(|s| s.as_str()).collect();
            let act: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
            let f = f1(&exp, &act);
            f1s.push(f);
            precisions.push(precision(&exp, &act));
            recalls.push(recall(&exp, &act));
            if f >= 0.5 {
                passed += 1;
            }
        }
    }

    Ok(LeaderEntry {
        retriever: retriever.name().to_string(),
        f1: if is_ranked { 0.0 } else { mean(&f1s) },
        recall: if is_ranked { 0.0 } else { mean(&recalls) },
        precision: if is_ranked { 0.0 } else { mean(&precisions) },
        mrr: if is_ranked { mean(&mrrs) } else { 0.0 },
        p95_latency_ms: p95(&latencies_ms),
        pass_rate: if gt.tasks.is_empty() {
            1.0
        } else {
            passed as f64 / gt.tasks.len() as f64
        },
    })
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn p95(xs: &[u64]) -> u64 {
    if xs.is_empty() {
        return 0;
    }
    let mut s = xs.to_vec();
    s.sort_unstable();
    let idx = ((s.len() as f64) * 0.95).ceil() as usize;
    let i = idx.saturating_sub(1).min(s.len() - 1);
    s[i]
}

fn default_hardware() -> String {
    // Bench-C2 — hardware string published alongside every leaderboard.
    std::env::var("GA_BENCH_HARDWARE")
        .unwrap_or_else(|_| format!("{} (local)", std::env::consts::ARCH))
}
