//! `graphatlas bench` CLI handler. Kept out of `main.rs` so the bin file
//! stays under the monorepo line-budget.

use anyhow::Result;

pub fn cmd_bench(
    uc: Option<String>,
    fixture: String,
    retrievers: Option<String>,
    gate: Option<String>,
    refresh_gt: bool,
    include_tests: bool,
) -> Result<()> {
    if matches!(gate.as_deref(), Some("m3")) {
        return cmd_bench_m3(uc, fixture, retrievers, refresh_gt);
    }
    let uc = uc.ok_or_else(|| {
        anyhow::anyhow!(
            "--uc required; one of: callers, callees, importers, symbols, file_summary, impact"
        )
    })?;
    let known = [
        "callers",
        "callees",
        "importers",
        "symbols",
        "file_summary",
        "impact",
    ];
    if !known.contains(&uc.as_str()) {
        return Err(anyhow::anyhow!(
            "unknown UC `{uc}` — supported: {}",
            known.join(", ")
        ));
    }
    let repo_root = std::env::current_dir()?;
    let layout = ga_bench::runner::UcLayout::for_uc(&repo_root, &uc, &fixture);

    if refresh_gt {
        return refresh_gt_cmd(&repo_root, &uc, &fixture, &layout, !include_tests);
    }
    let cache_root = repo_root.join(".graphatlas-bench-cache").join(&uc);
    // Reset cache each run so every bench measures a cold index (Bench-C2
    // hardware baseline is only meaningful against a clean build).
    let _ = std::fs::remove_dir_all(&cache_root);
    let opts = ga_bench::runner::RunOpts {
        uc: uc.clone(),
        fixture_dir: layout.fixture_dir.clone(),
        gt_path: layout.gt_path.clone(),
        cache_root: cache_root.clone(),
        out_md: layout.out_md.clone(),
    };
    let lb = match retrievers.as_deref() {
        None => ga_bench::runner::run_uc(opts),
        Some(csv) => {
            let names: Vec<&str> = csv
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            let set = ga_bench::runner::build_retrievers(&names, cache_root)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ga_bench::runner::run_uc_with(opts, set)
        }
    }
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "graphatlas bench --uc {uc}: wrote {}\n  fixture: {}\n  hardware: {}",
        layout.out_md.display(),
        layout.fixture_dir.display(),
        lb.hardware
    );
    for e in &lb.entries {
        println!(
            "  {:<14} F1={:.3} R={:.3} P={:.3} MRR={:.3} p95={}ms pass={:.0}%",
            e.retriever,
            e.f1,
            e.recall,
            e.precision,
            e.mrr,
            e.p95_latency_ms,
            e.pass_rate * 100.0
        );
    }
    Ok(())
}

/// AS-001/002/004 — `--gate m3` dispatch. Phase 1a: run() returns empty rows
/// because no rule is wired; the dispatcher still emits a leaderboard md for
/// audit and exits cleanly. S-004+ replaces empty rows with real scoring.
fn cmd_bench_m3(
    uc: Option<String>,
    fixture: String,
    retrievers: Option<String>,
    refresh_gt: bool,
) -> Result<()> {
    let uc = uc.ok_or_else(|| {
        anyhow::anyhow!(
            "--uc required for --gate m3; one of: dead_code, rename_safety, minimal_context, architecture, risk"
        )
    })?;
    if refresh_gt {
        // refresh-gt for M3 needs a rule to actually produce GT — wired in S-004+.
        return Err(anyhow::anyhow!(
            "--refresh-gt --gate m3 --uc {uc}: no rule wired yet; lands in S-004+ per graphatlas-v1.1-bench.md"
        ));
    }
    let retriever_list: Vec<String> = retrievers
        .as_deref()
        .map(|csv| {
            csv.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let repo_root = std::env::current_dir()?;
    let output_dir = repo_root.join("bench-results");
    let outcome = match ga_bench::m3_runner::run_m3_cli(&output_dir, &uc, &fixture, &retriever_list)
    {
        Ok(o) => o,
        Err(ga_bench::BenchError::UnknownM3Uc(_)) => {
            // AS-002.T1 — exit 2 + clear stderr.
            eprintln!(
                "unknown M3 UC `{uc}`; valid: dead_code|rename_safety|minimal_context|architecture|risk"
            );
            std::process::exit(2);
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };

    println!(
        "graphatlas bench --gate m3 --uc {uc}: wrote {} ({} rows)",
        outcome.leaderboard_path.display(),
        outcome.rows.len()
    );
    if outcome.exit_code != 0 {
        // AS-004.T4 — non-zero exit on FAIL, after writing leaderboard.
        std::process::exit(outcome.exit_code);
    }
    Ok(())
}

fn refresh_gt_cmd(
    repo_root: &std::path::Path,
    uc: &str,
    fixture: &str,
    layout: &ga_bench::runner::UcLayout,
    exclude_tests: bool,
) -> Result<()> {
    use ga_bench::gt_gen::{default_rules_with, generate_gt, to_pretty_json};
    use ga_index::Store;
    use ga_query::indexer::build_index;

    if !layout.fixture_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "fixture directory missing: {}. Check `git submodule update --init --recursive`.",
            layout.fixture_dir.display()
        ));
    }
    let cache_root = repo_root
        .join(".graphatlas-bench-cache")
        .join(format!("refresh-gt-{uc}-{fixture}"));
    let _ = std::fs::remove_dir_all(&cache_root);
    let store = Store::open_with_root(&cache_root, &layout.fixture_dir)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    build_index(&store, &layout.fixture_dir).map_err(|e| anyhow::anyhow!("build_index: {e}"))?;

    let rules = default_rules_with(exclude_tests);
    let gt = generate_gt(uc, fixture, &store, &layout.fixture_dir, &rules)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let out_path = repo_root
        .join("benches")
        .join(format!("uc-{uc}"))
        .join(format!("{fixture}.generated.json"));
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = to_pretty_json(&gt).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&out_path, json)?;
    println!(
        "refresh-gt --uc {uc} --fixture {fixture}: wrote {} tasks → {}",
        gt.tasks.len(),
        out_path.display()
    );
    Ok(())
}
