//! M2 gate markdown leaderboard writer — per-fixture + aggregate.
//!
//! Output layout (written into `RunOpts.output_dir`):
//!   impact-<repo>-leaderboard.md   (5 files, one per repo)
//!   impact-aggregate.md             (cross-repo rollup + tagline)

use crate::m2_runner::{M2Report, RepoGroup, RetrieverEntry};
use crate::BenchError;
use std::fs;
use std::path::Path;

pub fn write_reports(report: &M2Report, out_dir: &Path) -> Result<(), BenchError> {
    fs::create_dir_all(out_dir)?;

    for repo in &report.per_repo {
        let md = render_fixture(repo, report);
        let path = out_dir.join(format!("impact-{}-leaderboard.md", repo.repo));
        fs::write(&path, md)?;
    }

    let agg = render_aggregate(report);
    fs::write(out_dir.join("impact-aggregate.md"), agg)?;

    Ok(())
}

fn star_rank(values: &[f64], this: f64) -> &'static str {
    // Top value gets ⭐ if strictly greater than the rest (within tolerance)
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if (this - max).abs() < 1e-6 && values.iter().filter(|v| (**v - max).abs() < 1e-6).count() == 1
    {
        " ⭐"
    } else {
        ""
    }
}

fn render_fixture(repo: &RepoGroup, report: &M2Report) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Leaderboard: UC `impact` — {}\n\n", repo.repo));
    md.push_str(&format!("**Language:** {}\n\n", repo.lang));
    md.push_str(&format!(
        "**Tasks:** {} (split: {})\n\n",
        repo.task_count, report.split
    ));
    md.push_str(&format!("**Spec:** {}\n\n", report.spec));
    md.push_str(&format!("**GT source:** {}\n\n", report.gt_source));
    md.push_str("**Gate:** composite ≥ 0.80 (S-004 AS-009)\n\n");

    // Collect values per column for ⭐ highlighting
    let entries: Vec<&RetrieverEntry> = repo.per_retriever.values().collect();
    let composites: Vec<f64> = entries.iter().map(|e| e.mean_composite).collect();
    let test_recalls: Vec<f64> = entries.iter().map(|e| e.mean_test_recall).collect();
    let completes: Vec<f64> = entries.iter().map(|e| e.mean_completeness).collect();
    let depths: Vec<f64> = entries.iter().map(|e| e.mean_depth_f1).collect();
    let precisions: Vec<f64> = entries.iter().map(|e| e.mean_precision).collect();
    let brcs: Vec<f64> = entries
        .iter()
        .map(|e| e.mean_blast_radius_coverage)
        .collect();
    let adjs: Vec<f64> = entries.iter().map(|e| e.mean_adjusted_precision).collect();

    md.push_str("| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |\n");
    md.push_str("|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|\n");

    let mut sorted: Vec<&RetrieverEntry> = entries.clone();
    sorted.sort_by(|a, b| {
        b.mean_composite
            .partial_cmp(&a.mean_composite)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for e in sorted {
        md.push_str(&format!(
            "| {:<9} | {:.3}{:<3} | {:.3}{:<3}   | {:.3}{:<3}    | {:.3}{:<3} | {:.3}{:<3}   | {:<6} | {:>6.1}%  | {:.3}{:<3}     | {:.3}{:<3} |\n",
            e.retriever,
            e.mean_composite, star_rank(&composites, e.mean_composite),
            e.mean_test_recall, star_rank(&test_recalls, e.mean_test_recall),
            e.mean_completeness, star_rank(&completes, e.mean_completeness),
            e.mean_depth_f1, star_rank(&depths, e.mean_depth_f1),
            e.mean_precision, star_rank(&precisions, e.mean_precision),
            e.p95_latency_ms,
            e.pass_rate * 100.0,
            e.mean_blast_radius_coverage, star_rank(&brcs, e.mean_blast_radius_coverage),
            e.mean_adjusted_precision, star_rank(&adjs, e.mean_adjusted_precision),
        ));
    }

    md.push_str("\n**Reproduce:** `graphatlas bench --uc impact --repo ");
    md.push_str(&repo.repo);
    md.push_str("`\n");
    md
}

fn render_aggregate(report: &M2Report) -> String {
    let mut md = String::new();
    md.push_str("# Impact Benchmark — Cross-Fixture Aggregate\n\n");
    md.push_str(&format!(
        "**Dataset:** {} tasks across {} repos ({})\n\n",
        report.total_tasks,
        report.per_repo.len(),
        report
            .per_repo
            .iter()
            .map(|r| r.repo.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    ));
    md.push_str(&format!("**Split:** {}\n\n", report.split));
    md.push_str(&format!("**Spec:** {}\n\n", report.spec));
    md.push_str(&format!("**GT source:** {}\n\n", report.gt_source));
    md.push_str("**Gate (AS-009):** composite ≥ 0.80 | test_recall ≥ 0.85 | completeness ≥ 0.80 | depth_F1 ≥ 0.80 | precision ≥ 0.70 | p95 ≤ 500ms\n\n");

    md.push_str("## Overall\n\n");

    let composites: Vec<f64> = report.aggregate.iter().map(|e| e.mean_composite).collect();
    let test_recalls: Vec<f64> = report
        .aggregate
        .iter()
        .map(|e| e.mean_test_recall)
        .collect();
    let completes: Vec<f64> = report
        .aggregate
        .iter()
        .map(|e| e.mean_completeness)
        .collect();
    let precisions: Vec<f64> = report.aggregate.iter().map(|e| e.mean_precision).collect();
    let brcs: Vec<f64> = report
        .aggregate
        .iter()
        .map(|e| e.mean_blast_radius_coverage)
        .collect();
    let adjs: Vec<f64> = report
        .aggregate
        .iter()
        .map(|e| e.mean_adjusted_precision)
        .collect();

    md.push_str("| Retriever | Composite | Test Recall | Completeness | Precision | p95 ms | Pass Rate | Gate | BlastRadius | AdjPrec |\n");
    md.push_str("|-----------|-----------|-------------|--------------|-----------|--------|-----------|------|-------------|----------|\n");

    let mut sorted: Vec<&RetrieverEntry> = report.aggregate.iter().collect();
    sorted.sort_by(|a, b| {
        b.mean_composite
            .partial_cmp(&a.mean_composite)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for e in &sorted {
        let gate = if e.mean_composite >= 0.80 {
            "✅"
        } else {
            "—"
        };
        md.push_str(&format!(
            "| {:<9} | {:.3}{:<3} | {:.3}{:<3}   | {:.3}{:<3}    | {:.3}{:<3}   | {:<6} | {:>6.1}%  | {} | {:.3}{:<3}     | {:.3}{:<3} |\n",
            e.retriever,
            e.mean_composite, star_rank(&composites, e.mean_composite),
            e.mean_test_recall, star_rank(&test_recalls, e.mean_test_recall),
            e.mean_completeness, star_rank(&completes, e.mean_completeness),
            e.mean_precision, star_rank(&precisions, e.mean_precision),
            e.p95_latency_ms,
            e.pass_rate * 100.0,
            gate,
            e.mean_blast_radius_coverage, star_rank(&brcs, e.mean_blast_radius_coverage),
            e.mean_adjusted_precision, star_rank(&adjs, e.mean_adjusted_precision),
        ));
    }

    // Token-cost (efficiency). Public framing: GA vs BM25 — the IR floor.
    // CRG/CM/CGC are intentionally omitted from this section; CRG is
    // measured under a different task contract (per-commit, not per-symbol)
    // and post-hoc adapter shims aren't apples-to-apples here. The accuracy
    // table above shows all retrievers; this table is the targeted
    // "structure vs lexical IR" comparison.
    let ga = report.aggregate.iter().find(|e| e.retriever == "ga");
    let bm25 = report.aggregate.iter().find(|e| e.retriever == "bm25");
    md.push_str("\n## Token cost vs lexical IR baseline\n\n");
    if let (Some(ga), Some(bm25)) = (ga, bm25) {
        let reach_ratio = if bm25.pct_reached_100 > 0.0 {
            ga.pct_reached_100 / bm25.pct_reached_100
        } else {
            0.0
        };
        let token_savings = if bm25.mean_tokens_to_100_when_reached > 0.0 {
            1.0 - (ga.mean_tokens_to_100_when_reached / bm25.mean_tokens_to_100_when_reached)
        } else {
            0.0
        };
        md.push_str(&format!(
            "**GA vs BM25:** resolves {:.2}× more regression-causing changes ({:.1}% vs {:.1}% reach 100% recall) using {:.0}% fewer tokens per successful retrieval ({:.0} vs {:.0}).\n\n",
            reach_ratio,
            ga.pct_reached_100 * 100.0,
            bm25.pct_reached_100 * 100.0,
            token_savings * 100.0,
            ga.mean_tokens_to_100_when_reached,
            bm25.mean_tokens_to_100_when_reached,
        ));
    }
    md.push_str("Token cost = bytes/4 of files an agent reads, walking the retriever's ranked list, to reach the recall threshold. Means are **conditional on success** — failures aren't folded in, since a retriever that returns fewer files would otherwise look cheaper just for missing more.\n\n");
    md.push_str("| Retriever | reached 50% | tokens→50% (when reached) | reached 100% | tokens→100% (when reached) | files returned |\n");
    md.push_str("|-----------|------------:|--------------------------:|-------------:|---------------------------:|---------------:|\n");
    let mut tc_sorted: Vec<&RetrieverEntry> = report
        .aggregate
        .iter()
        .filter(|e| matches!(e.retriever.as_str(), "ga" | "bm25" | "ripgrep" | "random"))
        .collect();
    tc_sorted.sort_by(|a, b| {
        b.pct_reached_100
            .partial_cmp(&a.pct_reached_100)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for e in &tc_sorted {
        md.push_str(&format!(
            "| {:<9} | {:>10.1}% | {:>25.0} | {:>11.1}% | {:>26.0} | {:>14.1} |\n",
            e.retriever,
            e.pct_reached_50 * 100.0,
            e.mean_tokens_to_50_when_reached,
            e.pct_reached_100 * 100.0,
            e.mean_tokens_to_100_when_reached,
            e.mean_files_returned,
        ));
    }

    // Auto-generated tagline
    md.push_str("\n## Summary\n\n");
    if let Some(top) = sorted.first() {
        let comp_leader = sorted.iter().max_by(|a, b| {
            a.mean_composite
                .partial_cmp(&b.mean_composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let prec_leader = sorted.iter().max_by(|a, b| {
            a.mean_precision
                .partial_cmp(&b.mean_precision)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let recall_leader = sorted.iter().max_by(|a, b| {
            a.mean_test_recall
                .partial_cmp(&b.mean_test_recall)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        md.push_str(&format!(
            "**Top composite:** `{}` ({:.3})\n\n",
            comp_leader
                .map(|e| e.retriever.as_str())
                .unwrap_or(top.retriever.as_str()),
            comp_leader.map(|e| e.mean_composite).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "**Top precision:** `{}` ({:.3})\n\n",
            prec_leader.map(|e| e.retriever.as_str()).unwrap_or("?"),
            prec_leader.map(|e| e.mean_precision).unwrap_or(0.0),
        ));
        md.push_str(&format!(
            "**Top test recall:** `{}` ({:.3})\n\n",
            recall_leader.map(|e| e.retriever.as_str()).unwrap_or("?"),
            recall_leader.map(|e| e.mean_test_recall).unwrap_or(0.0),
        ));
    }

    md.push_str("\n## Per-repo composite\n\n");
    md.push_str("| Repo | Lang | Tasks |");
    // collect unique retrievers
    let mut retrievers: Vec<String> = report
        .aggregate
        .iter()
        .map(|e| e.retriever.clone())
        .collect();
    retrievers.sort();
    for r in &retrievers {
        md.push_str(&format!(" {} |", r));
    }
    md.push('\n');
    md.push_str("|------|------|-------|");
    for _ in &retrievers {
        md.push_str("-------|");
    }
    md.push('\n');

    for repo in &report.per_repo {
        md.push_str(&format!(
            "| {} | {} | {} |",
            repo.repo, repo.lang, repo.task_count
        ));
        for r in &retrievers {
            let c = repo
                .per_retriever
                .get(r)
                .map(|e| e.mean_composite)
                .unwrap_or(0.0);
            md.push_str(&format!(" {:.3} |", c));
        }
        md.push('\n');
    }

    md.push_str("\n**Reproduce:** `graphatlas bench --uc impact`\n");
    md.push_str("\n**Methodology:** see [docs/guide/uc-impact-dataset-methodology.md](../docs/guide/uc-impact-dataset-methodology.md)\n");
    md
}
