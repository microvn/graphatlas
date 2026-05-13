//! S-001 ga_risk — standalone composite risk.rs unit + integration tests.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-001):
//!   AS-001: Risk score happy path — 4-dim composite, score in [0,1],
//!     level low|medium|high, reasons array.
//!   AS-002: Fresh symbol (0 callers) — neutral test_gap, score < 0.4.
//!   AS-003: changed_files mode — max per-file risk + meta.per_file.
//!   AS-004: Symbol not found — typed Err with Levenshtein suggestions.
//!
//! Composite formula per Tools-C2 (pinned):
//!   0.4·test_gap + 0.3·blast_radius + 0.15·blame_churn + 0.15·bug_correlation

use ga_index::Store;
use ga_query::blame::BlameMiner;
use ga_query::indexer::build_index;
use ga_query::risk::{risk, RiskLevel, RiskRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ─────────────────────────────────────────────────────────────────────────
// Test stub miner (mirrors ga_risk_blame.rs pattern)
// ─────────────────────────────────────────────────────────────────────────

struct StubMiner {
    log: std::collections::HashMap<String, Vec<String>>,
}

impl StubMiner {
    fn empty() -> Self {
        Self {
            log: std::collections::HashMap::new(),
        }
    }
    fn with(file: &str, subjects: Vec<&str>) -> Self {
        let mut log = std::collections::HashMap::new();
        log.insert(
            file.to_string(),
            subjects.into_iter().map(String::from).collect(),
        );
        Self { log }
    }
}

impl BlameMiner for StubMiner {
    fn commit_subjects_since(&self, file: &str, _days: u32) -> Vec<String> {
        self.log.get(file).cloned().unwrap_or_default()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Test fixture builder — minimal indexed Python repo
// ─────────────────────────────────────────────────────────────────────────

fn setup() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (tmp, cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn build_repo_with_callers(
    repo: &Path,
    seed_file: &str,
    seed_symbol: &str,
    n_callers: usize,
    n_tests: usize,
) {
    write(
        &repo.join(seed_file),
        &format!("def {seed_symbol}(x):\n    return x * 2\n"),
    );
    for i in 0..n_callers {
        write(
            &repo.join(format!("caller_{i}.py")),
            &format!(
                "from {} import {seed_symbol}\n\ndef caller_{i}():\n    {seed_symbol}(1)\n",
                seed_file.trim_end_matches(".py")
            ),
        );
    }
    for i in 0..n_tests {
        write(
            &repo.join(format!("test_{seed_symbol}_{i}.py")),
            &format!(
                "from {} import {seed_symbol}\n\ndef test_{seed_symbol}_{i}():\n    assert {seed_symbol}(1) == 2\n",
                seed_file.trim_end_matches(".py")
            ),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AS-001 — Risk happy path
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn risk_score_for_indexed_symbol_in_zero_one_range() {
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "compute", 4, 2);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::with(
        "core.py",
        vec!["fix: edge case", "feat: new flag", "fix: rounding"],
    );
    let req = RiskRequest::for_symbol("compute");
    let resp = risk(&store, &miner, &req).expect("risk happy path");

    assert!(
        resp.score >= 0.0 && resp.score <= 1.0,
        "score must be in [0, 1]; got {}",
        resp.score
    );
    assert!(matches!(
        resp.level,
        RiskLevel::Low | RiskLevel::Medium | RiskLevel::High
    ));
}

#[test]
fn risk_response_includes_per_dim_breakdown() {
    // AS-001 implicit: response should expose how each dim contributed so
    // LLM can inspect WHY the score is what it is.
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "compute", 4, 2);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::with("core.py", vec!["fix: a", "feat: b"]);
    let req = RiskRequest::for_symbol("compute");
    let resp = risk(&store, &miner, &req).unwrap();

    let dim = &resp.meta.per_dim;
    assert!(dim.test_gap >= 0.0 && dim.test_gap <= 1.0);
    assert!(dim.blast_radius >= 0.0 && dim.blast_radius <= 1.0);
    assert!(dim.blame_churn >= 0.0 && dim.blame_churn <= 1.0);
    assert!(dim.bug_correlation >= 0.0 && dim.bug_correlation <= 1.0);
}

#[test]
fn risk_reasons_non_empty_for_symbol_with_signal() {
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "compute", 4, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::with(
        "core.py",
        vec!["fix: a", "fix: b", "fix: c", "fix: d", "fix: e"],
    );
    let req = RiskRequest::for_symbol("compute");
    let resp = risk(&store, &miner, &req).unwrap();

    assert!(
        !resp.reasons.is_empty(),
        "callers=4 + tests=0 + 5 fix-commits should yield ≥1 reason"
    );
}

#[test]
fn composite_formula_weights_pinned_per_tools_c2() {
    // Regression guard for Tools-C2: weights MUST be 0.4/0.3/0.15/0.15.
    // Construct a scenario where each dim is at known value and verify
    // composite = 0.4·1.0 + 0.3·0.5 + 0.15·1.0 + 0.15·1.0 = 0.7
    // by saturating all dims via fixture choice + miner stub.
    use ga_query::risk::compose_score;

    let s = compose_score(1.0, 0.5, 1.0, 1.0);
    assert!(
        (s - (0.4 + 0.15 + 0.15 + 0.15)).abs() < 1e-5,
        "0.4·1 + 0.3·0.5 + 0.15·1 + 0.15·1 = 0.85; got {s}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-002 — Fresh symbol (0 callers) returns low risk
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn fresh_symbol_no_callers_no_history_yields_low_risk() {
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "new_helper", 0, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::empty(); // no git history
    let req = RiskRequest::for_symbol("new_helper");
    let resp = risk(&store, &miner, &req).unwrap();

    assert!(
        resp.score < 0.4,
        "fresh-symbol AS-002 score must stay low (<0.4); got {}",
        resp.score
    );
    assert_eq!(resp.level, RiskLevel::Low);
}

#[test]
fn zero_callers_uses_neutral_test_gap_not_max() {
    // AS-002 spec: "Test gap dim treated as neutral (0.5) when callers=0
    // to avoid false-positive high risk on deletions."
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "lonely", 0, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::empty();
    let req = RiskRequest::for_symbol("lonely");
    let resp = risk(&store, &miner, &req).unwrap();

    let test_gap = resp.meta.per_dim.test_gap;
    assert!(
        (test_gap - 0.5).abs() < 1e-5,
        "AS-002: callers=0 → test_gap = neutral 0.5; got {test_gap}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-003 — changed_files mode: max per-file risk + meta.per_file
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn changed_files_mode_returns_max_per_file_risk() {
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "low.py", "low_fn", 0, 0);
    build_repo_with_callers(&repo, "high.py", "high_fn", 8, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::with(
        "high.py",
        vec!["fix: a", "fix: b", "fix: c", "fix: d", "bug: e"],
    );
    let req = RiskRequest::for_changed_files(vec!["low.py".into(), "high.py".into()]);
    let resp = risk(&store, &miner, &req).unwrap();

    let per_file = &resp.meta.per_file;
    assert_eq!(per_file.len(), 2, "meta.per_file: 1 entry per changed file");
    let high = per_file.get("high.py").expect("high.py per-file score");
    let low = per_file.get("low.py").expect("low.py per-file score");
    assert!(
        *high > *low,
        "high.py with 8 callers + 5 bug-fixes should score higher than low.py: high={high} low={low}"
    );
    let max = high.max(*low);
    assert!(
        (resp.score - max).abs() < 1e-5,
        "AS-003: union score = max per-file score; max={max}, got resp.score={}",
        resp.score
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-004 — Symbol not found → Err with Levenshtein suggestions
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn nonexistent_symbol_returns_invalid_params_with_suggestions() {
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "core.py", "compute", 1, 0);
    build_repo_with_callers(&repo, "util.py", "compose", 1, 0);
    build_repo_with_callers(&repo, "extra.py", "comprehension", 1, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::empty();
    let req = RiskRequest::for_symbol("compyte"); // typo of "compute"
    let result = risk(&store, &miner, &req);

    let err = result.expect_err("AS-004 must Err on unknown symbol");
    use ga_core::Error;
    match err {
        Error::SymbolNotFound { suggestions } => {
            assert!(
                !suggestions.is_empty(),
                "AS-004 suggestions array must be non-empty"
            );
            assert!(
                suggestions.iter().any(|s| s == "compute" || s == "compose"),
                "AS-004 suggestions must include nearest matches; got: {suggestions:?}"
            );
            assert!(
                suggestions.len() <= 3,
                "AS-004 suggestions capped at 3; got {} entries",
                suggestions.len()
            );
        }
        other => panic!("expected SymbolNotFound (structured); got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_request_neither_symbol_nor_changed_files_errs() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def a():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::empty();
    let req = RiskRequest::default();
    let result = risk(&store, &miner, &req);
    assert!(result.is_err(), "empty request must Err per Tools-C1");
}

#[test]
fn changed_files_with_unknown_paths_returns_zero_risk_not_error() {
    // AS-003 graceful — unknown file path doesn't break the call; it just
    // contributes 0 to the union (no callers, no history).
    let (_tmp, cache, repo) = setup();
    build_repo_with_callers(&repo, "real.py", "fn", 1, 0);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let miner = StubMiner::empty();
    let req = RiskRequest::for_changed_files(vec!["never_indexed.py".into()]);
    let resp = risk(&store, &miner, &req).expect("unknown file is graceful");
    assert_eq!(resp.score, 0.0);
    assert_eq!(resp.level, RiskLevel::Low);
}
