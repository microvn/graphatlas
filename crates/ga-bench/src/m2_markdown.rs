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
