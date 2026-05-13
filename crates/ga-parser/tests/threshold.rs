//! S-004 AS-011 + AS-012 — per-language parse-failure threshold.
//!
//! Continue through <=30% failures (warning). Abort with typed error once
//! > 30%. Tracked per language so one broken grammar doesn't stop others.

use ga_core::Lang;
use ga_parser::threshold::{LangStats, ThresholdOutcome, DEFAULT_THRESHOLD};

#[test]
fn default_threshold_is_30_percent() {
    assert!((DEFAULT_THRESHOLD - 0.30).abs() < f32::EPSILON);
}

#[test]
fn under_30_percent_returns_continue() {
    // 100 files, 5 failed → 5% < 30% → Continue.
    let mut s = LangStats::new(Lang::Python);
    for _ in 0..95 {
        s.record_ok();
    }
    for _ in 0..5 {
        s.record_failure();
    }
    match s.evaluate(DEFAULT_THRESHOLD) {
        ThresholdOutcome::Continue { failure_rate, .. } => {
            assert!(failure_rate < 0.10);
        }
        other => panic!("expected Continue, got {other:?}"),
    }
}

#[test]
fn at_exactly_30_percent_continues() {
    // Boundary: 100 total, 30 failed → exactly 30%. Inclusive threshold.
    let mut s = LangStats::new(Lang::Python);
    for _ in 0..70 {
        s.record_ok();
    }
    for _ in 0..30 {
        s.record_failure();
    }
    assert!(matches!(
        s.evaluate(DEFAULT_THRESHOLD),
        ThresholdOutcome::Continue { .. }
    ));
}

#[test]
fn over_30_percent_aborts_with_spec_literal() {
    // 10 TypeScript files, 4 failed → 40% > 30% → AbortBrokenGrammar.
    let mut s = LangStats::new(Lang::TypeScript);
    for _ in 0..6 {
        s.record_ok();
    }
    for _ in 0..4 {
        s.record_failure();
    }
    match s.evaluate(DEFAULT_THRESHOLD) {
        ThresholdOutcome::AbortBrokenGrammar {
            lang,
            failure_rate,
            threshold,
            message,
        } => {
            assert_eq!(lang, Lang::TypeScript);
            assert!((failure_rate - 0.40).abs() < 0.01);
            assert!((threshold - 0.30).abs() < f32::EPSILON);
            // Spec literal match per AS-012.
            assert!(
                message.contains("TypeScript")
                    && message.contains("parsing failed")
                    && message.contains("40%")
                    && message.contains("30% threshold")
                    && message.contains("broken grammar"),
                "message should match AS-012 literal, got: {message}"
            );
        }
        other => panic!("expected AbortBrokenGrammar, got {other:?}"),
    }
}

#[test]
fn empty_stats_is_continue_not_divide_by_zero() {
    let s = LangStats::new(Lang::Rust);
    assert!(matches!(
        s.evaluate(DEFAULT_THRESHOLD),
        ThresholdOutcome::Continue { .. }
    ));
}

#[test]
fn thresholds_are_per_language_independently() {
    // Python can be healthy while TypeScript is broken.
    let mut py = LangStats::new(Lang::Python);
    let mut ts = LangStats::new(Lang::TypeScript);
    for _ in 0..10 {
        py.record_ok();
    }
    for _ in 0..3 {
        ts.record_ok();
    }
    for _ in 0..7 {
        ts.record_failure();
    }
    assert!(matches!(
        py.evaluate(DEFAULT_THRESHOLD),
        ThresholdOutcome::Continue { .. }
    ));
    assert!(matches!(
        ts.evaluate(DEFAULT_THRESHOLD),
        ThresholdOutcome::AbortBrokenGrammar { .. }
    ));
}
