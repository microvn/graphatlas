//! Cluster C7 — 4-dim runtime risk composite.
//!
//! Takes the already-computed signals (impacted files, tests, routes, configs,
//! break points, meta) and composes a [`Risk`] in [0.0, 1.0] with a
//! user-facing level and a short list of the dominant risk drivers.
//!
//! Note on spec alignment: AS-012's "0.4·test_recall + 0.3·completeness +
//! 0.15·depth_F1 + 0.15·precision" composite is the **benchmark quality**
//! metric (measures how good `ga_impact` is at finding the truth) and lands
//! in C11 as a separate scorer. This module computes the **runtime risk**
//! (how dangerous is this change) — a different question with different
//! weights. Both are 4-dim by coincidence.

use super::types::{
    AffectedConfig, AffectedRoute, AffectedTest, BreakPoint, ImpactMeta, ImpactedFile, Risk,
    RiskLevel,
};

/// Saturation threshold — repos with more than this many impacted files
/// score 1.0 on the blast-radius dim.
const BLAST_SATURATION: f32 = 20.0;

/// Dims below this weighted contribution are dropped from the reasons list.
const REASON_THRESHOLD: f32 = 0.05;

/// Runtime risk composite.
///
/// Weights: `0.4·test_gap + 0.3·blast + 0.15·depth + 0.15·exposure`.
/// Each input dim is clamped to `[0.0, 1.0]` before weighting; the output
/// `score` is therefore also in `[0.0, 1.0]`.
pub(super) fn compute_risk(
    impacted_files: &[ImpactedFile],
    affected_tests: &[AffectedTest],
    affected_routes: &[AffectedRoute],
    affected_configs: &[AffectedConfig],
    break_points: &[BreakPoint],
    meta: &ImpactMeta,
) -> Risk {
    let test_gap = if break_points.is_empty() {
        0.0
    } else {
        let ratio = affected_tests.len() as f32 / break_points.len() as f32;
        (1.0 - ratio).clamp(0.0, 1.0)
    };
    let blast_factor = (impacted_files.len() as f32 / BLAST_SATURATION).min(1.0);
    let depth_factor = if meta.max_depth == 0 {
        0.0
    } else {
        (meta.transitive_completeness as f32 / meta.max_depth as f32).clamp(0.0, 1.0)
    };
    let exposure_factor = {
        let route = if affected_routes.is_empty() { 0.0 } else { 1.0 };
        let config = if affected_configs.is_empty() {
            0.0
        } else {
            1.0
        };
        0.5 * route + 0.5 * config
    };

    let test_contrib = 0.4 * test_gap;
    let blast_contrib = 0.3 * blast_factor;
    let depth_contrib = 0.15 * depth_factor;
    let exposure_contrib = 0.15 * exposure_factor;
    let score = (test_contrib + blast_contrib + depth_contrib + exposure_contrib).clamp(0.0, 1.0);

    let level = if score >= 0.7 {
        RiskLevel::High
    } else if score >= 0.4 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    let mut contribs: Vec<(f32, &'static str)> = vec![
        (test_contrib, "low test coverage"),
        (blast_contrib, "large blast radius"),
        (depth_contrib, "deep transitive propagation"),
        (exposure_contrib, "public API or config exposure"),
    ];
    contribs.retain(|(c, _)| *c >= REASON_THRESHOLD);
    contribs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let reasons: Vec<String> = contribs
        .into_iter()
        .take(3)
        .map(|(_, r)| r.to_string())
        .collect();

    Risk {
        score,
        level,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imp(n: usize) -> Vec<ImpactedFile> {
        (0..n)
            .map(|i| ImpactedFile {
                path: format!("f{i}.py"),
                depth: 1,
                reason: crate::impact::ImpactReason::Caller,
                ..Default::default()
            })
            .collect()
    }

    fn bp(n: usize) -> Vec<BreakPoint> {
        (0..n)
            .map(|i| BreakPoint {
                file: format!("c{i}.py"),
                line: 1,
                caller_symbols: vec!["c".into()],
            })
            .collect()
    }

    fn tests(n: usize) -> Vec<AffectedTest> {
        (0..n)
            .map(|i| AffectedTest {
                path: format!("test_{i}.py"),
                reason: crate::impact::AffectedTestReason::Convention,
            })
            .collect()
    }

    fn routes(n: usize) -> Vec<AffectedRoute> {
        (0..n)
            .map(|i| AffectedRoute {
                method: "GET".into(),
                path: format!("/r{i}"),
                source_file: "r.go".into(),
            })
            .collect()
    }

    fn configs(n: usize) -> Vec<AffectedConfig> {
        (0..n)
            .map(|i| AffectedConfig {
                path: format!("c{i}.yaml"),
                line: 1,
            })
            .collect()
    }

    fn meta(completeness: u32, max_depth: u32) -> ImpactMeta {
        ImpactMeta {
            transitive_completeness: completeness,
            max_depth,
            ..Default::default()
        }
    }

    #[test]
    fn empty_signals_yield_zero_low_no_reasons() {
        let r = compute_risk(&[], &[], &[], &[], &[], &meta(0, 0));
        assert_eq!(r.score, 0.0);
        assert_eq!(r.level, RiskLevel::Low);
        assert!(r.reasons.is_empty());
    }

    #[test]
    fn untested_large_blast_is_high() {
        // 40 impacted → blast saturates. 5 break points, 0 tests → test_gap=1.
        // routes + configs + deep chain → full saturation.
        let r = compute_risk(&imp(40), &[], &routes(1), &configs(1), &bp(5), &meta(3, 3));
        assert!(r.score >= 0.95, "score should saturate near 1.0: {r:?}");
        assert_eq!(r.level, RiskLevel::High);
    }

    #[test]
    fn fully_tested_small_change_is_low() {
        // 2 impacted files, 1 break point, 2 tests → test_gap = 0.
        let r = compute_risk(&imp(2), &tests(2), &[], &[], &bp(1), &meta(1, 3));
        assert!(r.score < 0.4, "score should stay low: {r:?}");
        assert_eq!(r.level, RiskLevel::Low);
    }

    #[test]
    fn level_boundary_at_exactly_0_7_is_high() {
        // test_gap=1 → contrib 0.4; blast=1 → 0.3. Sum=0.7 exactly.
        let r = compute_risk(&imp(20), &[], &[], &[], &bp(1), &meta(0, 0));
        assert!((r.score - 0.7).abs() < 1e-4, "score {}", r.score);
        assert_eq!(r.level, RiskLevel::High);
    }

    #[test]
    fn level_boundary_at_exactly_0_4_is_medium() {
        // test_gap=1 → 0.4.
        let r = compute_risk(&imp(0), &[], &[], &[], &bp(1), &meta(0, 0));
        assert!((r.score - 0.4).abs() < 1e-4);
        assert_eq!(r.level, RiskLevel::Medium);
    }

    #[test]
    fn score_clamped_to_one() {
        let r = compute_risk(
            &imp(1000),
            &[],
            &routes(100),
            &configs(100),
            &bp(1),
            &meta(10, 3),
        );
        assert!(r.score <= 1.0);
    }

    #[test]
    fn reasons_sorted_descending_by_contribution() {
        // test_gap=1 (0.4), blast=1 (0.3), depth=1 (0.15), exposure=1 (0.15).
        let r = compute_risk(&imp(20), &[], &routes(1), &configs(1), &bp(1), &meta(3, 3));
        assert_eq!(r.reasons.len(), 3); // capped at 3
        assert_eq!(r.reasons[0], "low test coverage");
        assert_eq!(r.reasons[1], "large blast radius");
        // third is either depth or exposure (tied at 0.15); both acceptable.
        assert!(
            r.reasons[2] == "deep transitive propagation"
                || r.reasons[2] == "public API or config exposure"
        );
    }

    #[test]
    fn reasons_below_threshold_dropped() {
        // Only test_gap contributes.
        let r = compute_risk(&[], &[], &[], &[], &bp(1), &meta(0, 0));
        assert_eq!(r.reasons, vec!["low test coverage".to_string()]);
    }

    #[test]
    fn reason_count_capped_at_three() {
        // All 4 dims contribute; reasons vec must have exactly 3 entries.
        let r = compute_risk(&imp(20), &[], &routes(1), &configs(1), &bp(1), &meta(3, 3));
        assert_eq!(r.reasons.len(), 3);
    }

    #[test]
    fn test_gap_zero_when_no_break_points() {
        // No call sites → coverage dim is moot → test_gap 0.
        let r = compute_risk(&imp(20), &[], &[], &[], &[], &meta(3, 3));
        // score = 0.3*1 + 0.15*1 + 0 = 0.45
        assert!((r.score - 0.45).abs() < 1e-4, "score {}", r.score);
    }

    #[test]
    fn depth_factor_zero_when_max_depth_zero() {
        let r = compute_risk(&imp(20), &[], &[], &[], &bp(1), &meta(3, 0));
        // only test_gap (0.4) + blast (0.3) = 0.7
        assert!((r.score - 0.7).abs() < 1e-4);
    }

    #[test]
    fn exposure_only_routes_half_weight() {
        let r = compute_risk(&[], &[], &routes(1), &[], &[], &meta(0, 0));
        // exposure_factor = 0.5 → contrib = 0.15 * 0.5 = 0.075
        assert!((r.score - 0.075).abs() < 1e-4, "score {}", r.score);
    }

    #[test]
    fn exposure_both_routes_and_configs_full_weight() {
        let r = compute_risk(&[], &[], &routes(1), &configs(1), &[], &meta(0, 0));
        // exposure_factor = 1.0 → contrib = 0.15
        assert!((r.score - 0.15).abs() < 1e-4);
    }
}
