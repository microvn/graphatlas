//! S-001 ga_risk — blame.rs subprocess wrapper unit tests (RED phase).
//!
//! Spec contract (graphatlas-v1.1-tools.md AS-001 §Data):
//!   "Commit history mined via git log on seed file over last 90 days.
//!    Bug correlation = % commits with keywords fix|bug|error|crash|regression
//!    in message."
//!
//! These tests use the `BlameMiner` trait so production code spawns
//! `git log` (real subprocess) but tests inject a deterministic stub —
//! avoids spawning git in CI + makes assertions reproducible.

use ga_query::blame::{BlameMiner, BlameStats};

// ─────────────────────────────────────────────────────────────────────────
// Test stub: in-memory commit log injected as fake git history
// ─────────────────────────────────────────────────────────────────────────

struct StubMiner {
    /// Pre-canned commit subjects per file path. None == file unknown to git.
    log: std::collections::HashMap<String, Option<Vec<String>>>,
}

impl StubMiner {
    fn with(file: &str, subjects: Vec<&str>) -> Self {
        let mut log = std::collections::HashMap::new();
        log.insert(
            file.to_string(),
            Some(subjects.into_iter().map(String::from).collect()),
        );
        Self { log }
    }
    fn unknown_file() -> Self {
        Self {
            log: std::collections::HashMap::new(),
        }
    }
}

impl BlameMiner for StubMiner {
    fn commit_subjects_since(&self, file: &str, _days: u32) -> Vec<String> {
        self.log.get(file).cloned().flatten().unwrap_or_default()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AS-001 §Data — happy path: blame_churn (commits/90d) + bug_correlation
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_history_yields_zero_churn_zero_bug_correlation() {
    let miner = StubMiner::unknown_file();
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 0);
    assert_eq!(stats.bug_fix_count, 0);
    assert_eq!(stats.churn(), 0.0);
    assert_eq!(stats.bug_correlation(), 0.0);
}

#[test]
fn churn_normalizes_against_saturation() {
    // Churn saturates at some threshold (≥ 20 commits/90d → 1.0). Concrete
    // saturation value pinned by impl; AS-001 example shows 8 commits → score
    // ~0.5 in mid range. Test the saturation contract: unbounded large counts
    // clamp to 1.0, not exceed it.
    let miner = StubMiner::with(
        "src/foo.py",
        std::iter::repeat_n("docs: tweak", 100).collect(),
    );
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 100);
    assert!(
        stats.churn() <= 1.0 && stats.churn() >= 0.99,
        "100 commits/90d should saturate at ~1.0, got {}",
        stats.churn()
    );
}

#[test]
fn bug_correlation_counts_fix_bug_error_crash_regression_keywords() {
    // AS-001 §Data: keywords = fix|bug|error|crash|regression (case-insensitive).
    let miner = StubMiner::with(
        "src/foo.py",
        vec![
            "fix: NPE on empty input",         // fix ✓
            "feat: add new endpoint",          // (not a bug-fix)
            "BUG: race condition in cache",    // bug ✓ (case-insensitive)
            "perf: optimize hot path",         // (not a bug-fix)
            "regression: revert flaky test",   // regression ✓
            "error: handle 500 from upstream", // error ✓
            "fixes #123: timeout reset",       // fixes (matches "fix" substring) ✓
            "docs: clarify API contract",      // (not a bug-fix)
        ],
    );
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 8);
    assert_eq!(
        stats.bug_fix_count, 5,
        "fix + BUG + regression + error + fixes = 5"
    );
    let expected = 5.0 / 8.0;
    assert!(
        (stats.bug_correlation() - expected).abs() < 1e-6,
        "bug_correlation = bug_fix_count / commit_count, expected {expected}, got {}",
        stats.bug_correlation()
    );
}

#[test]
fn churn_mid_range_value_for_8_commits() {
    // AS-001 example: 8 commits in 90d → churn moderate. With saturation = 20
    // commits, 8/20 = 0.4. Pin this so future re-tuning of saturation surfaces.
    let miner = StubMiner::with("src/foo.py", std::iter::repeat_n("docs", 8).collect());
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 8);
    let churn = stats.churn();
    assert!(
        churn > 0.3 && churn < 0.6,
        "8 commits should yield mid-range churn (saturation tunable but in 0.3..0.6); got {churn}"
    );
}

#[test]
fn bug_correlation_keywords_are_case_insensitive() {
    let miner = StubMiner::with(
        "src/foo.py",
        vec!["FIX: caps", "Bug: title-case", "ERROR boundary leak"],
    );
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.bug_fix_count, 3);
}

#[test]
fn non_bug_keywords_do_not_match() {
    // "feat", "chore", "docs", "perf", "refactor", "test", "style" → not bug-fix.
    let miner = StubMiner::with(
        "src/foo.py",
        vec![
            "feat: add API",
            "chore: bump deps",
            "docs: README polish",
            "perf: hot path",
            "refactor: extract helper",
            "test: more cases",
            "style: rustfmt",
        ],
    );
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 7);
    assert_eq!(stats.bug_fix_count, 0);
    assert_eq!(stats.bug_correlation(), 0.0);
}

#[test]
fn keyword_substring_match_is_word_boundary_aware() {
    // "prefix" contains "fix" but is NOT a bug-fix keyword. We require word
    // boundary so common false-positives (prefix, suffix, fixture, errors-out
    // in non-defect contexts) don't inflate bug_correlation.
    let miner = StubMiner::with(
        "src/foo.py",
        vec![
            "feat: add prefix-stripping helper", // contains "fix" substring → must NOT count
            "test: improve fixtures organization", // contains "fix" → must NOT count
            "fix: real bug fix",                 // word "fix" → counts
        ],
    );
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 3);
    assert_eq!(
        stats.bug_fix_count, 1,
        "only the third commit is a real bug-fix; prefix/fixture must not match"
    );
}

#[test]
fn commit_count_unaffected_by_subject_content() {
    // commit_count tracks raw history depth — different signal from bug ratio.
    let miner = StubMiner::with("src/foo.py", vec!["chore", "feat", "fix", "refactor"]);
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert_eq!(stats.commit_count, 4);
    assert_eq!(stats.bug_fix_count, 1);
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_file_yields_empty_stats() {
    // No git history for the file (e.g., file outside repo, file added in
    // working tree but not committed yet, or git binary missing).
    let miner = StubMiner::unknown_file();
    let stats = BlameStats::compute(&miner, "src/never_committed.py", 90);
    assert_eq!(stats.commit_count, 0);
    assert_eq!(stats.bug_fix_count, 0);
    assert_eq!(stats.churn(), 0.0);
    assert_eq!(stats.bug_correlation(), 0.0);
}

#[test]
fn churn_clamped_to_one() {
    // Adversarial: file with extreme churn (e.g., 1000 commits) must still
    // produce churn ≤ 1.0 (saturation contract).
    let miner = StubMiner::with("src/foo.py", std::iter::repeat_n("touch", 1000).collect());
    let stats = BlameStats::compute(&miner, "src/foo.py", 90);
    assert!(stats.churn() <= 1.0);
}
