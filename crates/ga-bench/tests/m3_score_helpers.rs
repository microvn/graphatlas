//! S-004 cycle 1 — pure scoring helpers for Hmc-budget UC.
//!
//! Per spec:
//! - AS-012.T1: file-recall against `must_touch_files`.
//! - AS-012.T2: secondary metrics — `test_recall`, `recall_per_1k_tokens`,
//!   `truncation_correctness_rate`.
//!
//! These helpers are pure functions over already-extracted file lists +
//! token counts; no fixture, store, or retriever needed.

use ga_bench::m3_score::{
    file_recall, recall_per_1k_tokens, test_recall, truncation_correctness_rate,
};

#[test]
fn as_012_t1_file_recall_perfect_match_is_one() {
    let actual = vec!["a.py".to_string(), "b.py".to_string()];
    let must = vec!["a.py".to_string(), "b.py".to_string()];
    assert_eq!(file_recall(&actual, &must), 1.0);
}

#[test]
fn as_012_t1_file_recall_partial_match() {
    let actual = vec!["a.py".to_string()];
    let must = vec!["a.py".to_string(), "b.py".to_string()];
    assert!((file_recall(&actual, &must) - 0.5).abs() < 1e-9);
}

#[test]
fn as_012_t1_file_recall_no_overlap_is_zero() {
    let actual = vec!["x.py".to_string()];
    let must = vec!["a.py".to_string(), "b.py".to_string()];
    assert_eq!(file_recall(&actual, &must), 0.0);
}

#[test]
fn as_012_t1_file_recall_empty_must_touch_is_one_when_actual_empty() {
    // No must-touch ⇒ trivially "all required files retrieved".
    assert_eq!(file_recall(&[], &[]), 1.0);
}

#[test]
fn as_012_t1_file_recall_empty_must_touch_is_one_regardless_of_actual() {
    let actual = vec!["noise.py".to_string()];
    assert_eq!(
        file_recall(&actual, &[]),
        1.0,
        "vacuous must-set should not penalize for extra returns; precision is a separate metric"
    );
}

#[test]
fn as_012_t1_file_recall_dedupes_actual() {
    let actual = vec!["a.py".to_string(), "a.py".to_string(), "a.py".to_string()];
    let must = vec!["a.py".to_string(), "b.py".to_string()];
    assert!(
        (file_recall(&actual, &must) - 0.5).abs() < 1e-9,
        "duplicate hits must not inflate recall"
    );
}

#[test]
fn as_012_t2_test_recall_filters_to_test_paths() {
    // Returned files = mix of prod + test. test_recall measures only the
    // test-must-touch subset.
    let actual = vec![
        "django/contrib/auth/models.py".to_string(),
        "tests/auth_tests/test_models.py".to_string(),
    ];
    let must_test = vec!["tests/auth_tests/test_models.py".to_string()];
    assert_eq!(test_recall(&actual, &must_test), 1.0);
}

#[test]
fn as_012_t2_test_recall_zero_when_test_missed() {
    let actual = vec!["django/contrib/auth/models.py".to_string()];
    let must_test = vec!["tests/auth_tests/test_models.py".to_string()];
    assert_eq!(test_recall(&actual, &must_test), 0.0);
}

#[test]
fn as_012_t2_recall_per_1k_tokens_normalizes_score_by_tokens() {
    // 0.7 file-recall over 2000 tokens → 0.35 recall/1k.
    let r = recall_per_1k_tokens(0.7, 2000);
    assert!((r - 0.35).abs() < 1e-9);
}

#[test]
fn as_012_t2_recall_per_1k_tokens_handles_zero_tokens() {
    // Zero tokens used (e.g. retriever returned empty within budget) ⇒
    // efficiency is not defined; convention: return 0.0 to keep
    // aggregations safe (NaN would poison means downstream).
    assert_eq!(recall_per_1k_tokens(0.7, 0), 0.0);
    assert_eq!(recall_per_1k_tokens(0.0, 0), 0.0);
}

#[test]
fn as_012_t2_truncation_correctness_means_truncated_iff_overflowing() {
    // Pairs of (truncated_flag, would_have_overflowed_budget). Correct iff
    // both agree. 4 cases: TT, TF, FT, FF → 2 correct out of 4 = 0.5.
    let pairs = vec![
        (true, true),   // correctly truncated (over budget)
        (true, false),  // wrongly truncated (was within budget)
        (false, true),  // wrongly NOT truncated (silently exceeded)
        (false, false), // correctly not truncated
    ];
    let r = truncation_correctness_rate(&pairs);
    assert!((r - 0.5).abs() < 1e-9, "got {r}");
}

#[test]
fn as_012_t2_truncation_correctness_empty_returns_one() {
    // Empty task list ⇒ vacuously perfect; aggregation-safe.
    assert_eq!(truncation_correctness_rate(&[]), 1.0);
}

#[test]
fn as_012_t2_truncation_correctness_all_correct() {
    let pairs = vec![(true, true), (false, false), (true, true)];
    assert_eq!(truncation_correctness_rate(&pairs), 1.0);
}
