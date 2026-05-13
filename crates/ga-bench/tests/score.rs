//! Bench S-001 cluster A — scorer primitives for per-UC leaderboards.

use ga_bench::score::{f1, mrr, precision, recall};

#[test]
fn precision_recall_f1_perfect_match() {
    let expected = vec!["a", "b", "c"];
    let actual = vec!["a", "b", "c"];
    assert_eq!(precision(&expected, &actual), 1.0);
    assert_eq!(recall(&expected, &actual), 1.0);
    assert_eq!(f1(&expected, &actual), 1.0);
}

#[test]
fn precision_rewards_only_true_positives() {
    // expected = {a,b,c}; actual = {a,b,x,y} → 2 TP, 2 FP → precision = 2/4
    let expected = vec!["a", "b", "c"];
    let actual = vec!["a", "b", "x", "y"];
    assert!((precision(&expected, &actual) - 0.5).abs() < 1e-6);
}

#[test]
fn recall_captures_missing_items() {
    // expected = {a,b,c}; actual = {a} → 1 TP, 2 FN → recall = 1/3
    let expected = vec!["a", "b", "c"];
    let actual = vec!["a"];
    assert!((recall(&expected, &actual) - 1.0 / 3.0).abs() < 1e-6);
}

#[test]
fn f1_handles_empty_actual_as_zero() {
    let expected = vec!["a"];
    let actual: Vec<&str> = vec![];
    assert_eq!(f1(&expected, &actual), 0.0);
}

#[test]
fn f1_handles_empty_expected_as_one_when_actual_empty() {
    // Nothing expected, nothing returned — by convention treat as pass (1.0).
    let expected: Vec<&str> = vec![];
    let actual: Vec<&str> = vec![];
    assert_eq!(f1(&expected, &actual), 1.0);
}

#[test]
fn f1_handles_empty_expected_with_nonempty_actual_as_zero() {
    // Nothing expected but tool returned noise → precision = 0 → F1 = 0.
    let expected: Vec<&str> = vec![];
    let actual = vec!["a"];
    assert_eq!(f1(&expected, &actual), 0.0);
}

#[test]
fn mrr_target_at_first_position() {
    let ranked = vec!["UserSerializer", "OtherThing"];
    assert_eq!(mrr(&ranked, &"UserSerializer"), 1.0);
}

#[test]
fn mrr_target_at_third_position() {
    let ranked = vec!["a", "b", "UserSerializer"];
    assert!((mrr(&ranked, &"UserSerializer") - 1.0 / 3.0).abs() < 1e-6);
}

#[test]
fn mrr_target_absent() {
    let ranked = vec!["a", "b", "c"];
    assert_eq!(mrr(&ranked, &"UserSerializer"), 0.0);
}
