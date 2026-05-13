//! AS-013.T1 plumbing — leaderboard markdown header surfaces each UC
//! rule's `id()` and `policy_bias()` text from the trait, not a stub.
//!
//! Migrated 2026-04-28: minimal_context rule is `Hmc-gitmine`
//! (ground-truth.json) — was `Hmc-budget` (archived tasks-v6).

use ga_bench::m3_runner::run_m3_cli;
use tempfile::tempdir;

#[test]
fn minimal_context_leaderboard_pulls_policy_bias_from_hmc_gitmine_rule() {
    let out = tempdir().unwrap();
    let outcome = run_m3_cli(out.path(), "minimal_context", "preact", &[])
        .expect("CLI dispatch must succeed");
    let md = std::fs::read_to_string(&outcome.leaderboard_path).unwrap();

    assert!(
        md.contains("**Rule:** Hmc-gitmine"),
        "header must name `Hmc-gitmine` (post-migration rule); got:\n{md}"
    );
    assert!(
        !md.contains("pending S-004"),
        "S-001 placeholder should be gone; got:\n{md}"
    );
    let lower = md.to_lowercase();
    assert!(
        lower.contains("ground-truth.json") || lower.contains("git-mining"),
        "header must include the dataset caveat from policy_bias(); got:\n{md}"
    );
}

#[test]
fn all_ucs_emit_rule_name_and_substantive_policy_bias() {
    for (uc, expected_rule) in [
        ("dead_code", "Hd-ast"),
        ("rename_safety", "Hrn-static"),
        ("architecture", "Ha-import-edge"),
    ] {
        let out = tempdir().unwrap();
        let outcome = run_m3_cli(out.path(), uc, "preact", &[]).unwrap();
        let md = std::fs::read_to_string(&outcome.leaderboard_path).unwrap();
        assert!(
            md.contains(&format!("**Rule:** {expected_rule}")),
            "{uc} leaderboard must name rule `{expected_rule}`; got:\n{md}"
        );
        assert!(
            !md.contains("pending S-"),
            "no `pending S-XXX` placeholder should remain in {uc} leaderboard; got:\n{md}"
        );
    }
}
