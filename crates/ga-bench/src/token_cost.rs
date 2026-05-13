//! Token-cost metric — how many tokens an LLM agent must read to recover
//! the expected file set, ordered by retriever's ranking.
//!
//! Motivation: composite/recall measure *correctness*. Token-cost measures
//! *efficiency*. Two retrievers can both achieve 50% recall — one by
//! returning 5 tight files, the other by returning 500 noisy ones. From
//! an agent's perspective the first is 100× cheaper. This metric makes
//! that delta visible.
//!
//! Method: walk the retriever's ranked `actual_files` prefix-by-prefix.
//! For each prefix, sum the file size in tokens and track recall against
//! `expected_files`. Record the smallest prefix that crosses the recall
//! threshold.
//!
//! Token approximation: `ceil(bytes / 4)`. Matches tiktoken's rule-of-thumb
//! for English+code (cl100k_base averages ~3.8 bytes/token) within ~5%
//! across our 12 fixture languages. We do not depend on tiktoken — the
//! point is cross-retriever *ranking*, not absolute count.
//!
//! Missing files (file path returned but absent from `fixture_dir`):
//! counted as 0 tokens. Rationale: the retriever returned a phantom path,
//! so an agent following it reads nothing. This penalizes correctness
//! (recall stays low) without double-penalizing on cost.
//!
//! Failed-to-reach-threshold: cost = sum of *all* returned-file tokens.
//! Agent paid the full read budget and still missed → that's the honest
//! upper bound on what the retriever cost them.

use std::collections::HashSet;
use std::path::Path;

/// Per-task token-cost outcome for one retriever's ranked file list.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenCost {
    /// Smallest prefix (in tokens) that reaches ≥50% recall on expected_files.
    /// If 50% never reached, equals `total_returned_tokens`.
    pub tokens_to_50: u64,
    /// Smallest prefix (in tokens) that reaches 100% recall.
    /// If never reached, equals `total_returned_tokens`.
    pub tokens_to_100: u64,
    pub achieved_50: bool,
    pub achieved_100: bool,
    /// Sum of tokens across the full returned list — the agent's worst-case read.
    pub total_returned_tokens: u64,
    /// Files returned. Useful for sanity-checking and reporting density.
    pub files_returned: u32,
}

/// Compute token-cost from a ranked `actual_files` list against `expected_files`.
/// `fixture_dir` is the root the retriever indexed; file paths are resolved
/// relative to it. Files outside or non-existent contribute 0 tokens.
pub fn compute(
    actual_files: &[String],
    expected_files: &[String],
    fixture_dir: &Path,
) -> TokenCost {
    let expected: HashSet<&str> = expected_files.iter().map(|s| s.as_str()).collect();
    let target_50 = (expected.len() as f64 * 0.5).ceil() as usize;
    let target_100 = expected.len();

    let mut hits = 0usize;
    let mut running_tokens: u64 = 0;
    let mut tokens_to_50: Option<u64> = None;
    let mut tokens_to_100: Option<u64> = None;

    for path in actual_files {
        let cost = file_tokens(fixture_dir, path);
        running_tokens = running_tokens.saturating_add(cost);
        if expected.contains(path.as_str()) {
            hits += 1;
            if tokens_to_50.is_none() && hits >= target_50 && target_50 > 0 {
                tokens_to_50 = Some(running_tokens);
            }
            if tokens_to_100.is_none() && hits >= target_100 && target_100 > 0 {
                tokens_to_100 = Some(running_tokens);
            }
        }
    }

    let total_returned_tokens = running_tokens;

    // Edge case: expected_files empty → trivially achieved at 0 cost.
    let (t50, ach50) = if target_50 == 0 {
        (0, true)
    } else {
        match tokens_to_50 {
            Some(v) => (v, true),
            None => (total_returned_tokens, false),
        }
    };
    let (t100, ach100) = if target_100 == 0 {
        (0, true)
    } else {
        match tokens_to_100 {
            Some(v) => (v, true),
            None => (total_returned_tokens, false),
        }
    };

    TokenCost {
        tokens_to_50: t50,
        tokens_to_100: t100,
        achieved_50: ach50,
        achieved_100: ach100,
        total_returned_tokens,
        files_returned: actual_files.len() as u32,
    }
}

fn file_tokens(fixture_dir: &Path, rel_path: &str) -> u64 {
    let full = fixture_dir.join(rel_path);
    match std::fs::metadata(&full) {
        Ok(m) if m.is_file() => bytes_to_tokens(m.len()),
        _ => 0,
    }
}

/// `ceil(bytes / 4)`. See module docs for rationale.
#[inline]
pub fn bytes_to_tokens(bytes: u64) -> u64 {
    bytes.div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, bytes: usize) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn bytes_to_tokens_ceils() {
        assert_eq!(bytes_to_tokens(0), 0);
        assert_eq!(bytes_to_tokens(1), 1);
        assert_eq!(bytes_to_tokens(4), 1);
        assert_eq!(bytes_to_tokens(5), 2);
        assert_eq!(bytes_to_tokens(400), 100);
    }

    #[test]
    fn perfect_first_hit_zero_overhead() {
        // Retriever returns exactly the expected file as #1 — minimum possible cost.
        let td = TempDir::new().unwrap();
        write(td.path(), "a.rs", 400); // 100 tokens
        let cost = compute(
            &["a.rs".to_string()],
            &["a.rs".to_string()],
            td.path(),
        );
        assert_eq!(cost.tokens_to_50, 100);
        assert_eq!(cost.tokens_to_100, 100);
        assert!(cost.achieved_50 && cost.achieved_100);
        assert_eq!(cost.files_returned, 1);
        assert_eq!(cost.total_returned_tokens, 100);
    }

    #[test]
    fn noise_before_hit_inflates_cost() {
        // Retriever returns 3 noise files then the answer. tokens_to_100 must
        // include all 4.
        let td = TempDir::new().unwrap();
        write(td.path(), "n1.rs", 400);
        write(td.path(), "n2.rs", 400);
        write(td.path(), "n3.rs", 400);
        write(td.path(), "a.rs", 400);
        let actual = vec![
            "n1.rs".to_string(),
            "n2.rs".to_string(),
            "n3.rs".to_string(),
            "a.rs".to_string(),
        ];
        let expected = vec!["a.rs".to_string()];
        let cost = compute(&actual, &expected, td.path());
        assert_eq!(cost.tokens_to_100, 400);
        assert!(cost.achieved_100);
        assert_eq!(cost.total_returned_tokens, 400);
    }

    #[test]
    fn fifty_percent_threshold_stops_early() {
        // 4 expected; 50% target = 2 hits. Returned: hit, noise, hit, ...
        let td = TempDir::new().unwrap();
        for f in ["a.rs", "b.rs", "c.rs", "d.rs", "n1.rs", "n2.rs"] {
            write(td.path(), f, 400); // 100 tokens each
        }
        let actual = vec![
            "a.rs".to_string(),
            "n1.rs".to_string(),
            "b.rs".to_string(),
            "n2.rs".to_string(),
            "c.rs".to_string(),
            "d.rs".to_string(),
        ];
        let expected = vec![
            "a.rs".to_string(),
            "b.rs".to_string(),
            "c.rs".to_string(),
            "d.rs".to_string(),
        ];
        let cost = compute(&actual, &expected, td.path());
        // 50% reached after a.rs + n1.rs + b.rs = 300 tokens
        assert_eq!(cost.tokens_to_50, 300);
        // 100% reached at end: all 6 files = 600 tokens
        assert_eq!(cost.tokens_to_100, 600);
        assert!(cost.achieved_50 && cost.achieved_100);
    }

    #[test]
    fn never_reaches_threshold_uses_full_budget() {
        let td = TempDir::new().unwrap();
        write(td.path(), "n1.rs", 400);
        write(td.path(), "n2.rs", 400);
        let actual = vec!["n1.rs".to_string(), "n2.rs".to_string()];
        let expected = vec!["a.rs".to_string()];
        let cost = compute(&actual, &expected, td.path());
        assert!(!cost.achieved_50);
        assert!(!cost.achieved_100);
        assert_eq!(cost.tokens_to_50, 200);
        assert_eq!(cost.tokens_to_100, 200);
    }

    #[test]
    fn phantom_paths_cost_zero_dont_corrupt_total() {
        // Retriever fabricates a path that doesn't exist — counts as 0 tokens.
        // Test ensures the running total isn't poisoned and a real hit after
        // a phantom is still costed correctly.
        let td = TempDir::new().unwrap();
        write(td.path(), "a.rs", 400);
        let actual = vec!["ghost.rs".to_string(), "a.rs".to_string()];
        let expected = vec!["a.rs".to_string()];
        let cost = compute(&actual, &expected, td.path());
        assert_eq!(cost.tokens_to_100, 100);
        assert_eq!(cost.total_returned_tokens, 100);
    }

    #[test]
    fn empty_expected_is_trivially_satisfied() {
        let td = TempDir::new().unwrap();
        let cost = compute(&[], &[], td.path());
        assert!(cost.achieved_50 && cost.achieved_100);
        assert_eq!(cost.tokens_to_50, 0);
        assert_eq!(cost.tokens_to_100, 0);
    }

    #[test]
    fn empty_actual_with_nonempty_expected_fails_at_zero_cost() {
        let td = TempDir::new().unwrap();
        let cost = compute(&[], &["a.rs".to_string()], td.path());
        assert!(!cost.achieved_50);
        assert!(!cost.achieved_100);
        assert_eq!(cost.total_returned_tokens, 0);
        assert_eq!(cost.files_returned, 0);
    }

    #[test]
    fn ranking_matters_same_set_different_order() {
        // Two retrievers return the same file set; the one that ranks the
        // answer first should win on tokens_to_100.
        let td = TempDir::new().unwrap();
        write(td.path(), "a.rs", 400);
        write(td.path(), "b.rs", 400);
        write(td.path(), "c.rs", 400);
        let expected = vec!["c.rs".to_string()];
        let good = compute(
            &["c.rs".into(), "a.rs".into(), "b.rs".into()],
            &expected,
            td.path(),
        );
        let bad = compute(
            &["a.rs".into(), "b.rs".into(), "c.rs".into()],
            &expected,
            td.path(),
        );
        assert!(
            good.tokens_to_100 < bad.tokens_to_100,
            "{} vs {}",
            good.tokens_to_100,
            bad.tokens_to_100
        );
    }
}
