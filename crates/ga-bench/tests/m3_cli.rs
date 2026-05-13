//! S-001 cycle 3 — CLI dispatch for `--gate m3`.
//!
//! - AS-001.T2: gate dispatcher routes `--uc {dead_code|rename_safety|minimal_context|architecture}` to m3.
//! - AS-001.T3: leaderboard markdown lands at `bench-results/m3-<uc>-<fixture>-leaderboard.md`.
//! - AS-002.T1: unknown UC → exit code 2 + stderr listing valid UCs.
//! - AS-004.T4: any FAIL row → exit code 1; all PASS → exit code 0.

use ga_bench::m3_runner::{run_m3_cli, M3CliOutcome};
use tempfile::tempdir;

#[test]
fn as_001_t3_dispatcher_writes_leaderboard_md_for_minimal_context() {
    let out_dir = tempdir().unwrap();
    let outcome = run_m3_cli(out_dir.path(), "minimal_context", "preact", &[])
        .expect("valid UC + empty retrievers must succeed (Phase 1a stub)");

    assert_eq!(outcome.exit_code, 0, "no failures → exit 0");
    let expected = out_dir
        .path()
        .join("m3-minimal_context-preact-leaderboard.md");
    assert_eq!(
        outcome.leaderboard_path, expected,
        "leaderboard path must follow `m3-<uc>-<fixture>-leaderboard.md` convention"
    );
    assert!(expected.is_file(), "leaderboard markdown must be written");

    let md = std::fs::read_to_string(&expected).unwrap();
    assert!(
        md.contains("# M3 Gate"),
        "markdown must have header; got:\n{md}"
    );
    assert!(md.contains("**SPEC GATE: 0 pass, 0 fail (target: all pass)**"));
}

#[test]
fn as_001_t2_dispatcher_accepts_each_phase12_uc() {
    for uc in [
        "dead_code",
        "rename_safety",
        "minimal_context",
        "architecture",
    ] {
        let out_dir = tempdir().unwrap();
        let outcome = run_m3_cli(out_dir.path(), uc, "preact", &[])
            .unwrap_or_else(|e| panic!("UC `{uc}` must dispatch cleanly: {e}"));
        assert_eq!(
            outcome.exit_code, 0,
            "UC `{uc}` should yield exit 0 in stub mode"
        );
        assert!(
            outcome.leaderboard_path.is_file(),
            "UC `{uc}` should still write a leaderboard md (audit trail)"
        );
    }
}

#[test]
fn as_002_t1_unknown_uc_yields_exit_code_2_with_clear_message() {
    let out_dir = tempdir().unwrap();
    let result = run_m3_cli(out_dir.path(), "bogus_uc", "preact", &[]);
    let outcome: M3CliOutcome = match result {
        Err(e) => {
            // Surface the message — AS-002 mandates clear stderr.
            let msg = e.to_string();
            assert!(
                msg.contains("bogus_uc"),
                "message must echo the bad UC; got: {msg}"
            );
            assert!(msg.contains("dead_code"), "message must list valid UCs");
            assert!(
                msg.contains("risk"),
                "message must list `risk` (Phase 3 deferral lifted once Hr-text shipped); got: {msg}"
            );
            return;
        }
        Ok(o) => o,
    };
    assert_eq!(
        outcome.exit_code, 2,
        "unknown UC must yield exit code 2 (per AS-002), got: {:?}",
        outcome
    );
}

#[test]
fn as_001_t2_risk_uc_is_now_a_valid_uc() {
    // Pre-Phase 3, `risk` was rejected as DEFERRED. Hr-text rule + score
    // loop closed the gap; `risk` is now a regular valid UC like the
    // other four.
    let out_dir = tempdir().unwrap();
    let outcome = run_m3_cli(out_dir.path(), "risk", "preact", &[])
        .expect("`risk` UC must dispatch cleanly now that Hr-text rule + score_risk shipped");
    assert!(
        outcome.leaderboard_path.is_file(),
        "risk UC should still write a leaderboard md (audit trail) even with empty retrievers"
    );
    assert_eq!(
        outcome.exit_code, 0,
        "no failures with empty retriever set → exit 0"
    );
}
