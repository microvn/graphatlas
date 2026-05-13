//! M3 score helpers — pure functions over file lists + token budgets.
//!
//! Per spec AS-012:
//! - Primary metric for Hmc-budget UC: `file_recall`.
//! - Secondary: `test_recall`, `recall_per_1k_tokens`, `truncation_correctness_rate`.
//!
//! All helpers are pure — no Store, no fixture, no retriever. They take
//! pre-extracted slices so the scoring loop can stay framework-agnostic
//! (m3_runner glues them onto `ga_minimal_context` output in cycle B).

use std::collections::BTreeSet;

use ga_query::common::is_test_path;

/// AS-012.T1 — file recall against the must-touch set.
///
/// Returns `|actual_files ∩ must_touch_files| / |must_touch_files|`. Empty
/// `must_touch_files` returns `1.0` (vacuously satisfied; precision is a
/// separate metric and not penalised here).
pub fn file_recall(actual_files: &[String], must_touch_files: &[String]) -> f64 {
    if must_touch_files.is_empty() {
        return 1.0;
    }
    let actual: BTreeSet<&str> = actual_files.iter().map(String::as_str).collect();
    let hits = must_touch_files
        .iter()
        .filter(|f| actual.contains(f.as_str()))
        .count();
    hits as f64 / must_touch_files.len() as f64
}

/// AS-012.T2 — test-only recall.
///
/// Filters `must_touch_files` to test paths via the canonical
/// `ga_query::common::is_test_path`, then computes recall on that slice.
/// Empty test-must-set returns `1.0` (same convention as `file_recall`).
pub fn test_recall(actual_files: &[String], must_touch_files: &[String]) -> f64 {
    let must_test: Vec<String> = must_touch_files
        .iter()
        .filter(|f| is_test_path(f))
        .cloned()
        .collect();
    file_recall(actual_files, &must_test)
}

/// AS-012.T2 — efficiency: file-recall normalised per 1k tokens used.
///
/// `tokens_used == 0` ⇒ `0.0` (zero-cost retrievers don't get to claim
/// infinite efficiency; NaN would poison aggregate means).
pub fn recall_per_1k_tokens(file_recall_score: f64, tokens_used: u32) -> f64 {
    if tokens_used == 0 {
        return 0.0;
    }
    file_recall_score / (tokens_used as f64 / 1000.0)
}

/// AS-012.T2 — fraction of tasks where the retriever's `truncated` flag
/// agrees with the budget reality. `(reported_truncated, exceeded_budget)`
/// pairs from the scoring loop.
///
/// Correct iff both agree (true, true) or (false, false). Empty input ⇒
/// `1.0` (vacuously perfect — aggregation-safe).
pub fn truncation_correctness_rate(pairs: &[(bool, bool)]) -> f64 {
    if pairs.is_empty() {
        return 1.0;
    }
    let correct = pairs.iter().filter(|(a, b)| a == b).count();
    correct as f64 / pairs.len() as f64
}
