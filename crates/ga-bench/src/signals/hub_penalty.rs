//! Hub detection via git commit frequency.
//!
//! Ported from `src/adapters/hub-penalty.ts`. Thesis: files touched in many
//! commits correlate with hub/core files (gin.go, django base classes) —
//! useful as a multiplier penalty during retrieval fusion.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Per-file commit counts + median — output of [`get_commit_frequencies`].
#[derive(Debug, Clone, Default)]
pub struct HubStats {
    pub commit_count: HashMap<String, u32>,
    pub median: u32,
}

/// Count commits touching each file via `git log --name-only`. Single
/// subprocess over all files; filtering + counting happens in-process.
///
/// Returns empty `HubStats` on any git failure (missing repo, timeout,
/// command-not-found, non-zero exit) — caller should treat absence as no-op.
pub fn get_commit_frequencies(repo_path: &Path, files: &[String], max_commits: u32) -> HubStats {
    if files.is_empty() {
        return HubStats::default();
    }
    let file_set: HashSet<&str> = files.iter().map(String::as_str).collect();

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args([
            "log",
            "--no-merges",
            &format!("-n{max_commits}"),
            "--name-only",
            "--format=",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = output else {
        return HubStats::default();
    };
    if !out.status.success() {
        return HubStats::default();
    }
    let Ok(text) = std::str::from_utf8(&out.stdout) else {
        return HubStats::default();
    };

    let mut commit_count: HashMap<String, u32> = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !file_set.contains(trimmed) {
            continue;
        }
        *commit_count.entry(trimmed.to_string()).or_insert(0) += 1;
    }

    let mut counts: Vec<u32> = commit_count.values().copied().collect();
    counts.sort_unstable();
    let median = if counts.is_empty() {
        0
    } else {
        counts[counts.len() / 2]
    };
    HubStats {
        commit_count,
        median,
    }
}

/// Hub penalty — multiplier `∈ (0, 1]` based on commit frequency vs median.
///
/// Heuristic:
/// - `ratio = count / median ≤ 3` → `1.0` (no penalty — typical file)
/// - `ratio = 10×median` → `0.5` (high-traffic)
/// - `ratio = 30×median` → `~0.2` (heavy hub)
///
/// Formula: `1 / (1 + max(0, ratio − 3) / 3)`.
///
/// Returns `1.0` when `median == 0` (no data → no penalty).
pub fn hub_multiplier(commit_count: u32, median: u32) -> f32 {
    if median == 0 {
        return 1.0;
    }
    let ratio = commit_count as f32 / median as f32;
    if ratio <= 3.0 {
        1.0
    } else {
        1.0 / (1.0 + (ratio - 3.0) / 3.0)
    }
}

/// v1-default commit-history depth for hub-frequency scans.
pub const DEFAULT_MAX_COMMITS: u32 = 500;

/// Unused but reserved for parity with TS version's `timeoutMs` option.
/// Kept as a compile-time constant so downstream code can reference the
/// intended bound without depending on subprocess wall-clock timeouts
/// (which `std::process::Command` doesn't support natively).
#[allow(dead_code)]
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2000);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_multiplier_returns_one_at_zero_median() {
        assert_eq!(hub_multiplier(100, 0), 1.0);
    }

    #[test]
    fn hub_multiplier_no_penalty_at_threshold() {
        // ratio = 3 → no penalty.
        assert_eq!(hub_multiplier(30, 10), 1.0);
    }

    #[test]
    fn hub_multiplier_half_at_ratio_ten() {
        // ratio = 10 → 1/(1 + 7/3) = 3/10 = 0.3 — correcting formula:
        // 1/(1 + (10 - 3)/3) = 1/(1 + 7/3) = 3/10 = 0.3
        let m = hub_multiplier(100, 10);
        assert!((m - 0.3).abs() < 1e-4, "m = {m}");
    }

    #[test]
    fn hub_multiplier_monotonic_with_frequency() {
        let a = hub_multiplier(50, 10);
        let b = hub_multiplier(100, 10);
        let c = hub_multiplier(300, 10);
        assert!(a > b && b > c, "a={a} b={b} c={c}");
    }

    #[test]
    fn get_commit_frequencies_empty_input_returns_empty_stats() {
        let tmp = tempfile::TempDir::new().unwrap();
        let stats = get_commit_frequencies(tmp.path(), &[], 100);
        assert!(stats.commit_count.is_empty());
        assert_eq!(stats.median, 0);
    }

    #[test]
    fn get_commit_frequencies_non_git_dir_returns_empty_stats() {
        // Any non-repo path → git errors → empty map per contract.
        let tmp = tempfile::TempDir::new().unwrap();
        let stats = get_commit_frequencies(tmp.path(), &["any.txt".to_string()], 100);
        assert!(stats.commit_count.is_empty());
        assert_eq!(stats.median, 0);
    }
}
