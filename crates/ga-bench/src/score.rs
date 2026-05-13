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
    if expected.is_empty() && actual.is_empty() {
        return 1.0;
    }
    let p = precision(expected, actual);
    let r = recall(expected, actual);
    if p + r == 0.0 {
        0.0
    } else {
        2.0 * p * r / (p + r)
    }
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

    ImpactScore {
        test_recall,
        completeness,
        depth_f1,
        precision: precision_dim,
        composite,
        blast_radius_coverage,
        adjusted_precision,
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
