//! S-001 cycle 2 — AS-004 leaderboard markdown rendering.
//!
//! - AS-004.T2: rows with `spec_status: Fail` get `**FAIL**` bold prefix.
//! - AS-004.T3: footer `**SPEC GATE: <P> pass, <F> fail (target: all pass)**`.
//! - AS-004.T4: `has_failures` returns `true` iff any row is Fail (CLI exit
//!   code 1 hook). Tested separately via subprocess in cycle 3 — here we
//!   pin the helper so the CLI wiring has a stable signal source.

use ga_bench::m3_runner::{has_failures, render_leaderboard_md, M3LeaderboardRow, SpecStatus};
use std::collections::BTreeMap;

fn row(retriever: &str, score: f64, target: f64, status: SpecStatus) -> M3LeaderboardRow {
    M3LeaderboardRow {
        retriever: retriever.to_string(),
        fixture: "preact".to_string(),
        uc: "minimal_context".to_string(),
        score,
        secondary_metrics: BTreeMap::new(),
        spec_status: status,
        spec_target: target,
        p95_latency_ms: 12,
    }
}

#[test]
fn as_004_t2_failing_rows_marked_with_bold_fail_prefix() {
    let rows = vec![
        row("ga", 0.65, 0.70, SpecStatus::Fail),
        row("ripgrep", 0.78, 0.70, SpecStatus::Pass),
    ];
    let md = render_leaderboard_md(
        "minimal_context",
        "preact",
        "Hmc-budget",
        "policy bias text",
        &rows,
    );

    let ga_line = md
        .lines()
        .find(|l| l.contains("ga") && !l.contains("ripgrep"))
        .expect("ga row must appear in markdown");
    assert!(
        ga_line.contains("**FAIL**"),
        "FAIL row must contain bold **FAIL** prefix; line: {ga_line}"
    );
    let rg_line = md.lines().find(|l| l.contains("ripgrep")).unwrap();
    assert!(
        !rg_line.contains("**FAIL**"),
        "PASS row must NOT contain **FAIL** prefix; line: {rg_line}"
    );
}

#[test]
fn as_004_t3_footer_summary_shows_pass_fail_counts() {
    let rows = vec![
        row("ga", 0.65, 0.70, SpecStatus::Fail),
        row("cgc", 0.55, 0.70, SpecStatus::Fail),
        row("ripgrep", 0.78, 0.70, SpecStatus::Pass),
    ];
    let md = render_leaderboard_md("minimal_context", "preact", "Hmc-budget", "bias", &rows);

    assert!(
        md.contains("**SPEC GATE: 1 pass, 2 fail (target: all pass)**"),
        "footer must contain `**SPEC GATE: 1 pass, 2 fail (target: all pass)**`; markdown:\n{md}"
    );
}

#[test]
fn as_004_all_pass_run_emits_zero_fail_in_footer_and_no_fail_prefix() {
    let rows = vec![
        row("ga", 0.85, 0.70, SpecStatus::Pass),
        row("cgc", 0.92, 0.70, SpecStatus::Pass),
    ];
    let md = render_leaderboard_md("minimal_context", "preact", "Hmc-budget", "bias", &rows);

    assert!(md.contains("**SPEC GATE: 2 pass, 0 fail (target: all pass)**"));
    assert!(
        !md.contains("**FAIL**"),
        "all-PASS run must not include **FAIL** marker anywhere; markdown:\n{md}"
    );
}

#[test]
fn as_013_header_lists_rule_and_policy_bias() {
    let rows = vec![row("ga", 0.85, 0.70, SpecStatus::Pass)];
    let md = render_leaderboard_md(
        "minimal_context",
        "preact",
        "Hmc-budget",
        "tasks-v6 GT favors file-level retrievers — see methodology.md",
        &rows,
    );

    assert!(
        md.contains("**Rule:** Hmc-budget"),
        "header must name the rule; got:\n{md}"
    );
    assert!(
        md.contains("**Policy bias:** tasks-v6 GT favors file-level retrievers"),
        "header must include policy_bias() text once; got:\n{md}"
    );
    // Single source of truth — biased note must appear at most once
    let bias_count = md.matches("**Policy bias:**").count();
    assert_eq!(
        bias_count, 1,
        "policy bias must appear exactly once (single source of truth)"
    );
}

#[test]
fn as_004_t4_has_failures_returns_true_when_any_row_fails() {
    let any_fail = vec![
        row("ga", 0.85, 0.70, SpecStatus::Pass),
        row("cgc", 0.55, 0.70, SpecStatus::Fail),
    ];
    assert!(has_failures(&any_fail));

    let all_pass = vec![row("ga", 0.85, 0.70, SpecStatus::Pass)];
    assert!(!has_failures(&all_pass));

    let empty: Vec<M3LeaderboardRow> = vec![];
    assert!(
        !has_failures(&empty),
        "empty leaderboard ≠ failure (Phase 1a stub returns Ok([])) — exit code stays 0"
    );

    let only_tautology = vec![row("ga", 1.0, 0.0, SpecStatus::Tautological)];
    assert!(
        !has_failures(&only_tautology),
        "TAUTOLOGICAL is a warning per AS-020, not a hard fail — exit code stays 0 unless paired with FAIL"
    );
}
