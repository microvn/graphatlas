//! Scoring primitives used across UC bench drivers. Set-based F1 for
//! callers/callees/importers; MRR for symbol-search ranking.

use std::collections::HashSet;
use std::hash::Hash;

/// Precision = |expected ∩ actual| / |actual|.
/// Empty `actual` → 0.0 (no claim made).
pub fn precision<T: Eq + Hash>(expected: &[T], actual: &[T]) -> f64 {
    if actual.is_empty() {
        return 0.0;
    }
    let exp: HashSet<_> = expected.iter().collect();
    let tp = actual.iter().filter(|a| exp.contains(a)).count();
    tp as f64 / actual.len() as f64
}

/// Recall = |expected ∩ actual| / |expected|.
/// Empty `expected` → 1.0 (nothing to miss).
pub fn recall<T: Eq + Hash>(expected: &[T], actual: &[T]) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let act: HashSet<_> = actual.iter().collect();
    let tp = expected.iter().filter(|e| act.contains(e)).count();
    tp as f64 / expected.len() as f64
}

/// F1 harmonic mean. Conventions:
/// - both empty → 1.0 (trivially correct).
/// - expected empty, actual non-empty → 0.0 (precision drops to 0).
/// - actual empty, expected non-empty → 0.0 (recall 0).
pub fn f1<T: Eq + Hash>(expected: &[T], actual: &[T]) -> f64 {
    f_beta(expected, actual, 1.0)
}

/// F-beta harmonic mean. `β > 1` weights recall over precision; `β < 1`
/// weights precision over recall. Same edge-case conventions as `f1`:
/// - both empty → 1.0
/// - expected empty, actual non-empty → 0.0
/// - actual empty, expected non-empty → 0.0
pub fn f_beta<T: Eq + Hash>(expected: &[T], actual: &[T], beta: f64) -> f64 {
    if expected.is_empty() && actual.is_empty() {
        return 1.0;
    }
    let p = precision(expected, actual);
    let r = recall(expected, actual);
    let b2 = beta * beta;
    let denom = b2 * p + r;
    if denom == 0.0 {
        0.0
    } else {
        (1.0 + b2) * p * r / denom
    }
}

/// F2 score — recall weighted 2× precision. For UCs where missing a true
/// positive (false negative) is costlier than including extra noise
/// (false positive). Cross-tool graph-retriever value-add lens: agent
/// would rather see N+1 files (1 noise) than miss the 1 file needed.
///
/// Hard floor convention (applied at reporting layer, not here): tools
/// with precision < 0.5 are considered noise-dominated regardless of
/// F2 number — caller should DQ them at the leaderboard render step.
pub fn f2<T: Eq + Hash>(expected: &[T], actual: &[T]) -> f64 {
    f_beta(expected, actual, 2.0)
}

/// Reciprocal rank of `target` inside a ranked list. 0.0 if absent.
pub fn mrr<T: Eq>(ranked: &[T], target: &T) -> f64 {
    ranked
        .iter()
        .position(|x| x == target)
        .map(|idx| 1.0 / (idx as f64 + 1.0))
        .unwrap_or(0.0)
}

/// 4-dim composite quality score for `ga_impact` per AS-012 Data:
/// `composite = 0.4·test_recall + 0.3·completeness + 0.15·depth_f1 + 0.15·precision`.
///
/// Dims (composite):
/// - `test_recall`: fraction of `expected_tests` that appear in `actual_tests`.
/// - `completeness`: fraction of `expected_files` that appear in `actual_files`
///   (aka file-level recall).
/// - `depth_f1`: BFS reach signal. When both `actual_max_depth` and
///   `expected_max_depth` are known, scores whether BFS expanded far enough
///   (`min(actual/expected, 1.0)`), decoupled from the file set. When depth
///   labels are absent, falls back to file-set F1 as a proxy.
/// - `precision`: file-level precision against `expected_files`.
///
/// Supplementary dims (not in composite):
/// - `blast_radius_coverage`: recall against `should_touch_files` (structural
///   blast radius). 1.0 when `should_touch_files` is empty.
/// - `adjusted_precision`: precision against `expected_files ∪ should_touch_files`.
///   Higher than raw `precision` for retrievers that correctly return blast
///   radius files (like GA). 0.0 when `actual_files` is empty.
///
/// **Target**: composite ≥0.80 per AS-012 benchmark criterion.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ImpactScore {
    pub test_recall: f64,
    pub completeness: f64,
    pub depth_f1: f64,
    pub precision: f64,
    pub composite: f64,
    // Supplementary — do not affect composite formula:
    pub blast_radius_coverage: f64,
    pub adjusted_precision: f64,
    /// F2 score on `(expected_files, actual_files)` — recall-priority lens.
    /// Reported as secondary diagnostic; the gate threshold stays on
    /// `composite ≥ 0.80`. Defaults to 0 for legacy entries deserialized
    /// before the field existed.
    #[serde(default)]
    pub f2_files: f64,
    /// F2 score on `(expected_tests, actual_tests)` — recall-priority on the
    /// test dimension. Secondary diagnostic; not in composite formula.
    #[serde(default)]
    pub f2_tests: f64,
}

pub fn impact_score(
    actual_files: &[String],
    actual_tests: &[String],
    actual_max_depth: Option<u32>,
    expected_files: &[String],
    expected_tests: &[String],
    expected_max_depth: Option<u32>,
    should_touch_files: &[String],
) -> ImpactScore {
    let test_recall = recall(expected_tests, actual_tests);
    let completeness = recall(expected_files, actual_files);
    let precision_dim = precision(expected_files, actual_files);
    let depth_f1 = depth_score(
        actual_files,
        expected_files,
        completeness,
        actual_max_depth,
        expected_max_depth,
    );
    let composite = 0.4 * test_recall + 0.3 * completeness + 0.15 * depth_f1 + 0.15 * precision_dim;

    // Supplementary: blast radius coverage + adjusted precision
    let blast_radius_coverage = recall(should_touch_files, actual_files);
    let mut adj_gt: std::collections::HashSet<String> = expected_files.iter().cloned().collect();
    adj_gt.extend(should_touch_files.iter().cloned());
    let adj_gt_vec: Vec<String> = adj_gt.into_iter().collect();
    let adjusted_precision = precision(&adj_gt_vec, actual_files);

    let f2_files = f2(expected_files, actual_files);
    let f2_tests = f2(expected_tests, actual_tests);

    ImpactScore {
        test_recall,
        completeness,
        depth_f1,
        precision: precision_dim,
        composite,
        blast_radius_coverage,
        adjusted_precision,
        f2_files,
        f2_tests,
    }
}

/// Depth-dim scorer. When GT labels `expected_max_depth` and the retriever
/// reports `actual_max_depth`, awards `min(actual/expected, 1.0)` — pure
/// reach signal, decoupled from file-set precision. Guards against the
/// degenerate "reached nothing but reports max_depth=N" case by returning 0
/// when no expected file was found (completeness == 0). Falls back to the
/// legacy file-F1 proxy when either label is absent so existing callers
/// with no depth metadata keep scoring the same way.
fn depth_score(
    actual_files: &[String],
    expected_files: &[String],
    completeness: f64,
    actual_max_depth: Option<u32>,
    expected_max_depth: Option<u32>,
) -> f64 {
    match (actual_max_depth, expected_max_depth) {
        (Some(_), Some(_)) if completeness == 0.0 => 0.0,
        (Some(a), Some(e)) => {
            if e == 0 {
                // Expected depth 0 = seed-only change. completeness > 0
                // already verified the seed was reached → full credit.
                1.0
            } else {
                (a as f64 / e as f64).min(1.0)
            }
        }
        _ => f1(expected_files, actual_files),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    // ──── F2 / F-beta primitives ────

    #[test]
    fn f2_balanced_perfect_equals_one() {
        assert!((f2(&s(&["a", "b"]), &s(&["a", "b"])) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn f2_both_empty_is_perfect() {
        let empty: Vec<String> = Vec::new();
        assert_eq!(f2(&empty, &empty), 1.0);
    }

    #[test]
    fn f2_recall_priority_vs_f1() {
        // Tool A: precision=1.0, recall=0.5 (missed half)
        // Tool B: precision=0.5, recall=1.0 (found all + noise)
        // Both have same F1 but F2 rewards B (recall priority).
        let exp = s(&["a", "b", "c", "d"]);
        let a_actual = s(&["a", "b"]); // missed c, d
        let b_actual = s(&["a", "b", "c", "d", "x", "y", "z", "w"]); // all + 4 noise
        let a_f1 = f1(&exp, &a_actual);
        let b_f1 = f1(&exp, &b_actual);
        let a_f2 = f2(&exp, &a_actual);
        let b_f2 = f2(&exp, &b_actual);
        // F1 symmetric: both around 0.66
        assert!(
            (a_f1 - b_f1).abs() < 0.05,
            "F1 should be similar: {a_f1} vs {b_f1}"
        );
        // F2: B (high recall) > A (high precision)
        assert!(b_f2 > a_f2, "F2 should reward recall: B={b_f2} A={a_f2}");
    }

    #[test]
    fn f2_punishes_zero_recall_harder_than_f1() {
        // P=1.0 R=0.1 → recall priority hurts F2 more.
        let exp = s(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let actual = s(&["a"]); // 1/10 recall, 1/1 precision
        let v_f1 = f1(&exp, &actual);
        let v_f2 = f2(&exp, &actual);
        assert!(
            v_f2 < v_f1,
            "F2 ({v_f2}) should be lower than F1 ({v_f1}) at low recall"
        );
    }

    #[test]
    fn f2_rewards_thorough_with_modest_noise() {
        // P=0.5, R=1.0 — full recall, half noise.
        // F1 = 0.667, F2 = 0.833 (recall priority)
        let exp = s(&["a", "b", "c"]);
        let actual = s(&["a", "b", "c", "x", "y", "z"]); // all + 3 noise
        let v_f1 = f1(&exp, &actual);
        let v_f2 = f2(&exp, &actual);
        assert!((v_f1 - 0.6667).abs() < 0.01, "F1 ≈ 0.667: {v_f1}");
        assert!((v_f2 - 0.8333).abs() < 0.01, "F2 ≈ 0.833: {v_f2}");
    }

    #[test]
    fn f_beta_extreme_recall_priority() {
        // β=4 — recall priority extreme. P=0.5, R=1.0 → F4 ≈ 0.94
        let exp = s(&["a", "b"]);
        let actual = s(&["a", "b", "x", "y"]);
        let v = f_beta(&exp, &actual, 4.0);
        assert!(v > 0.9, "F4 with full recall should be >0.9: {v}");
    }

    #[test]
    fn f_beta_zero_when_one_side_empty() {
        let exp: Vec<String> = s(&["a"]);
        let actual: Vec<String> = Vec::new();
        assert_eq!(f_beta(&exp, &actual, 2.0), 0.0);
        let exp2: Vec<String> = Vec::new();
        let actual2: Vec<String> = s(&["a"]);
        assert_eq!(f_beta(&exp2, &actual2, 2.0), 0.0);
    }

    #[test]
    fn f1_via_f_beta_equals_legacy_formula() {
        // Sanity: legacy f1 == f_beta(β=1).
        let exp = s(&["a", "b", "c"]);
        let actual = s(&["a", "b", "d"]); // P=2/3, R=2/3, F1=2/3
        let legacy = 2.0 * 0.6667 * 0.6667 / (0.6667 + 0.6667);
        let via_beta = f_beta(&exp, &actual, 1.0);
        let via_f1 = f1(&exp, &actual);
        assert!((via_beta - legacy).abs() < 0.001);
        assert!((via_f1 - legacy).abs() < 0.001);
    }

    // ──── impact_score (composite) — pre-existing ────

    #[test]
    fn impact_score_perfect_hit_is_one() {
        let r = impact_score(
            &s(&["a.py", "b.py"]),
            &s(&["test_a.py"]),
            None,
            &s(&["a.py", "b.py"]),
            &s(&["test_a.py"]),
            None,
            &s(&[]),
        );
        assert!((r.composite - 1.0).abs() < 1e-6, "{}", r.composite);
        assert_eq!(r.test_recall, 1.0);
        assert_eq!(r.completeness, 1.0);
        assert_eq!(r.precision, 1.0);
    }

    #[test]
    fn impact_score_miss_files_zero_composite() {
        let r = impact_score(
            &s(&[]),
            &s(&[]),
            None,
            &s(&["a.py"]),
            &s(&["test_a.py"]),
            None,
            &s(&[]),
        );
        // test_recall=0, completeness=0, precision=0 (actual empty), depth_f1=0.
        assert!(r.composite < 0.01, "{}", r.composite);
    }

    #[test]
    fn impact_score_no_expected_tests_weights_files_only() {
        // expected_tests empty → test_recall = 1.0 (nothing to miss).
        let r = impact_score(
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&[]),
        );
        assert_eq!(r.test_recall, 1.0);
        assert_eq!(r.completeness, 1.0);
        assert!((r.composite - 1.0).abs() < 1e-6);
    }

    #[test]
    fn impact_score_test_recall_dominates_weights() {
        // Only test_recall=0.5, everything else zero. composite = 0.4·0.5 = 0.2.
        let r = impact_score(
            &s(&[]),
            &s(&["test_a.py"]),
            None,
            &s(&["a.py"]),
            &s(&["test_a.py", "test_b.py"]),
            None,
            &s(&[]),
        );
        assert!(
            (r.composite - 0.2).abs() < 1e-6,
            "composite={}",
            r.composite
        );
    }

    #[test]
    fn impact_score_composite_equals_weighted_sum() {
        let r = impact_score(
            &s(&["a.py", "noise.py"]),
            &s(&["test_a.py"]),
            None,
            &s(&["a.py", "b.py"]),
            &s(&["test_a.py"]),
            None,
            &s(&[]),
        );
        let expected =
            0.4 * r.test_recall + 0.3 * r.completeness + 0.15 * r.depth_f1 + 0.15 * r.precision;
        assert!((r.composite - expected).abs() < 1e-6);
    }

    #[test]
    fn depth_dim_decoupled_from_file_set_when_labels_present() {
        // Noisy actual (low precision, 1 of 5 matches) but BFS reached far
        // enough (depth 3 ≥ expected 1). Legacy proxy would give
        // depth_f1 ≈ file-F1 ≈ 0.33; new scorer must return 1.0.
        let r = impact_score(
            &s(&["a.py", "n1.py", "n2.py", "n3.py", "n4.py"]),
            &s(&[]),
            Some(3),
            &s(&["a.py"]),
            &s(&[]),
            Some(1),
            &s(&[]),
        );
        assert!(
            (r.depth_f1 - 1.0).abs() < 1e-6,
            "depth_f1 must decouple from file-F1; got {}",
            r.depth_f1
        );
    }

    #[test]
    fn depth_dim_zero_when_no_expected_file_matched() {
        // Reached depth 3 but completeness = 0 → can't claim depth coverage.
        let r = impact_score(
            &s(&["noise.py"]),
            &s(&[]),
            Some(3),
            &s(&["a.py"]),
            &s(&[]),
            Some(1),
            &s(&[]),
        );
        assert_eq!(r.depth_f1, 0.0, "no matched file → depth_f1 must be 0");
    }

    #[test]
    fn depth_dim_proportional_when_under_reached() {
        // Reached depth 1 of expected 2 → 0.5.
        let r = impact_score(
            &s(&["a.py"]),
            &s(&[]),
            Some(1),
            &s(&["a.py"]),
            &s(&[]),
            Some(2),
            &s(&[]),
        );
        assert!((r.depth_f1 - 0.5).abs() < 1e-6, "got {}", r.depth_f1);
    }

    #[test]
    fn depth_dim_falls_back_to_f1_without_labels() {
        // No depth labels → legacy file-F1 proxy preserved for backward-compat.
        let r = impact_score(
            &s(&["a.py", "noise.py"]),
            &s(&[]),
            None,
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&[]),
        );
        let expected_f1 = f1(&s(&["a.py"]), &s(&["a.py", "noise.py"]));
        assert!((r.depth_f1 - expected_f1).abs() < 1e-6);
    }

    // --- Supplementary metric tests ---

    #[test]
    fn blast_radius_coverage_one_when_should_touch_empty() {
        // Empty should_touch_files → nothing to miss → recall = 1.0
        let r = impact_score(
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&[]),
        );
        assert_eq!(r.blast_radius_coverage, 1.0);
    }

    #[test]
    fn blast_radius_coverage_partial() {
        // should_touch = {b.py, c.py}, actual covers b.py only → 0.5
        let r = impact_score(
            &s(&["a.py", "b.py"]),
            &s(&[]),
            None,
            &s(&["a.py"]),
            &s(&[]),
            None,
            &s(&["b.py", "c.py"]),
        );
        assert!(
            (r.blast_radius_coverage - 0.5).abs() < 1e-6,
            "got {}",
            r.blast_radius_coverage
        );
    }

    #[test]
    fn adjusted_precision_higher_than_raw_when_blast_radius_returned() {
        // GA returns expected + blast radius. Raw precision penalises blast radius files.
        // adjusted_precision uses enlarged GT → no penalty.
        let r = impact_score(
            &s(&["a.py", "blast.py"]), // actual: expected + blast radius file
            &s(&[]),
            None,
            &s(&["a.py"]), // expected: only the commit-diff file
            &s(&[]),
            None,
            &s(&["blast.py"]), // should_touch: blast radius file
        );
        // raw precision = 1/2 = 0.5 (blast.py not in expected_files)
        assert!(
            (r.precision - 0.5).abs() < 1e-6,
            "raw precision={}",
            r.precision
        );
        // adjusted_precision = 2/2 = 1.0 (blast.py IS in expected ∪ should_touch)
        assert!(
            (r.adjusted_precision - 1.0).abs() < 1e-6,
            "adj precision={}",
            r.adjusted_precision
        );
        assert!(
            r.adjusted_precision > r.precision,
            "adjusted must exceed raw"
        );
    }

    #[test]
    fn existing_composite_unchanged_by_new_fields() {
        // Adding should_touch_files does not affect composite formula
        let r_no_stf = impact_score(
            &s(&["a.py", "b.py"]),
            &s(&["t.py"]),
            None,
            &s(&["a.py", "b.py"]),
            &s(&["t.py"]),
            None,
            &s(&[]),
        );
        let r_with_stf = impact_score(
            &s(&["a.py", "b.py"]),
            &s(&["t.py"]),
            None,
            &s(&["a.py", "b.py"]),
            &s(&["t.py"]),
            None,
            &s(&["c.py", "d.py"]),
        );
        assert!(
            (r_no_stf.composite - r_with_stf.composite).abs() < 1e-9,
            "composite must not change: {} vs {}",
            r_no_stf.composite,
            r_with_stf.composite
        );
        assert!(
            (r_no_stf.precision - r_with_stf.precision).abs() < 1e-9,
            "raw precision must not change"
        );
    }
}
