//! S-001 ga_risk — git-log mining for blame_churn + bug_correlation dims.
//!
//! Spec contract (graphatlas-v1.1-tools.md AS-001 §Data):
//!   "Commit history mined via git log on seed file over last 90 days.
//!    Bug correlation = % commits with keywords fix|bug|error|crash|regression
//!    in message."
//!
//! Architecture: subprocess-based (mirrors `scripts/mine-fix-commits.ts`
//! pattern) — no new Cargo dep. The [`BlameMiner`] trait abstracts the
//! source so tests can inject a deterministic stub without spawning git.
//!
//! Saturation: churn normalizes against [`CHURN_SATURATION`] commits/window.
//! 20 commits/90d → 1.0 (~ daily commit cadence on a hot file).

use std::path::Path;
use std::process::Command;

/// Commits/window above which churn dim saturates at 1.0.
/// Pinned at 20 — reflects "very active file" threshold (≥1 commit per
/// ~5 days over a 90-day window). Re-tuning requires Tools-C2 ADR.
pub const CHURN_SATURATION: u32 = 20;

/// Bug-fix keyword set. Case-insensitive whole-word match (regex `\b<kw>\b`).
/// Source: AS-001 §Data — `fix|bug|error|crash|regression`. Implementation
/// expands the literal spec set with common verb inflections so real-world
/// commit subjects like `fixes #123:` and `crashed on init` count without
/// requiring a stricter writer convention. The expansion does NOT change
/// composite weights (Tools-C2 unaffected) — it only widens detection
/// recall on the same definitional category. `prefix` / `fixture` /
/// `errored` (verb past on non-defect) etc. are kept out via strict
/// whole-word boundary check (see `matches_bug_keyword`).
pub const BUG_KEYWORDS: &[&str] = &[
    "fix",
    "fixed",
    "fixes",
    "fixing",
    "bug",
    "bugs",
    "error",
    "errors",
    "crash",
    "crashed",
    "crashes",
    "regression",
    "regressions",
];

/// Source of git commit history. Production uses [`GitLogMiner`]; tests
/// inject deterministic stubs.
pub trait BlameMiner {
    /// Return commit subjects (one per commit) touching `file` in the last
    /// `days` days. Empty vec when file unknown to git or repo missing.
    /// Anchored to wall-clock — production `ga_risk` semantics.
    fn commit_subjects_since(&self, file: &str, days: u32) -> Vec<String>;

    /// Same as [`commit_subjects_since`] but with the time window anchored
    /// to a specific commit (`anchor_ref`'s committer-date), not wall-clock.
    /// Used by the M3 bench `Hr-text` rule to keep scoring reproducible
    /// against frozen submodule fixtures: a fixture pinned to a 2024
    /// commit must mine the 90 days BEFORE that commit, not the 90 days
    /// before today.
    ///
    /// Default impl falls back to `commit_subjects_since` so existing
    /// `BlameMiner` implementors (StubMiner in tests) compile unchanged
    /// and the production `ga_risk` path is unaffected.
    fn commit_subjects_in_window(&self, file: &str, anchor_ref: &str, days: u32) -> Vec<String> {
        let _ = anchor_ref;
        self.commit_subjects_since(file, days)
    }
}

/// Production [`BlameMiner`] — spawns `git log` against `repo_root`.
///
/// Failure modes (all return `Vec::new()` per Tools-C1 graceful-degrade
/// posture; `ga_risk` callers see "no git history" reasons rather than
/// hard error):
/// - Repo path is not a git working tree (no `.git` dir/file)
/// - `git` binary missing on PATH
/// - File path outside repo or never committed
/// - git emits non-UTF8 (we drop those lines)
pub struct GitLogMiner {
    repo_root: std::path::PathBuf,
}

impl GitLogMiner {
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            repo_root: repo_root.as_ref().to_path_buf(),
        }
    }
}

impl BlameMiner for GitLogMiner {
    fn commit_subjects_since(&self, file: &str, days: u32) -> Vec<String> {
        let since = format!("{days}.days.ago");
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .arg("log")
            .arg(format!("--since={since}"))
            .arg("--pretty=format:%s")
            .arg("--no-merges")
            .arg("--")
            .arg(file)
            .output();
        let Ok(out) = output else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    /// M3 bench `Hr-text` reproducibility — anchor the time window to
    /// `anchor_ref`'s committer date instead of wall-clock. Two-step:
    /// resolve to a Unix timestamp via `%ct`, then bound `git log` via
    /// `--max-age` (= since) + `--min-age` (= until) which take Unix
    /// timestamps and avoid the date-arithmetic parsing trap that
    /// affected the cycle-B v1 implementation (`--since="<iso> - 90 days"`
    /// silently returned empty because `git log --since` doesn't parse
    /// arithmetic).
    fn commit_subjects_in_window(&self, file: &str, anchor_ref: &str, days: u32) -> Vec<String> {
        let ts_out = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .arg("show")
            .arg("-s")
            .arg("--format=%ct")
            .arg(anchor_ref)
            .output();
        let Ok(ts) = ts_out else {
            return Vec::new();
        };
        if !ts.status.success() {
            return Vec::new();
        }
        let anchor_unix: i64 = match String::from_utf8_lossy(&ts.stdout).trim().parse() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let since_unix = anchor_unix - (days as i64) * 86400;
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .arg("log")
            .arg(anchor_ref)
            .arg(format!("--max-age={since_unix}"))
            .arg(format!("--min-age={anchor_unix}"))
            .arg("--pretty=format:%s")
            .arg("--no-merges")
            .arg("--")
            .arg(file)
            .output();
        let Ok(out) = output else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }
}

/// Computed blame statistics for a single file over a time window.
#[derive(Debug, Clone, PartialEq)]
pub struct BlameStats {
    pub commit_count: u32,
    pub bug_fix_count: u32,
}

impl BlameStats {
    /// Mine + classify. `days` is the lookback window (typically 90 per
    /// AS-001). Wall-clock anchored — production `ga_risk` semantics.
    pub fn compute(miner: &impl BlameMiner, file: &str, days: u32) -> Self {
        let subjects = miner.commit_subjects_since(file, days);
        Self::from_subjects(subjects)
    }

    /// Same as [`compute`] but anchors the time window to `anchor_ref`'s
    /// committer-date instead of wall-clock. Used by the M3 bench harness
    /// against frozen fixture submodules: a fixture pinned to 2024-12 must
    /// mine the 90 days BEFORE that commit, not the 90 days before today.
    /// Without this, `ga_risk` returns 0 commit_count on every file in
    /// stale fixtures → bug_correlation drops to 0 → engine and GT use
    /// different time anchors → F1 floors at the test_gap+blast contribution.
    pub fn compute_in_window(
        miner: &impl BlameMiner,
        file: &str,
        anchor_ref: &str,
        days: u32,
    ) -> Self {
        let subjects = miner.commit_subjects_in_window(file, anchor_ref, days);
        Self::from_subjects(subjects)
    }

    fn from_subjects(subjects: Vec<String>) -> Self {
        let commit_count = subjects.len() as u32;
        let bug_fix_count = subjects.iter().filter(|s| matches_bug_keyword(s)).count() as u32;
        Self {
            commit_count,
            bug_fix_count,
        }
    }

    /// Normalized churn dim in `[0.0, 1.0]`. Saturates at
    /// [`CHURN_SATURATION`] commits/window.
    pub fn churn(&self) -> f32 {
        (self.commit_count as f32 / CHURN_SATURATION as f32).min(1.0)
    }

    /// Bug-fix ratio in `[0.0, 1.0]`. Returns 0.0 when commit_count == 0
    /// (no signal — neutral, not penalizing).
    pub fn bug_correlation(&self) -> f32 {
        if self.commit_count == 0 {
            return 0.0;
        }
        self.bug_fix_count as f32 / self.commit_count as f32
    }
}

/// Whole-word case-insensitive match against `BUG_KEYWORDS`.
///
/// Word-boundary check prevents `prefix`, `fixture`, `suffix` from
/// false-matching `fix` substring. We approximate `\b` by requiring
/// the matched run to be flanked by non-alphanumeric characters (or
/// the start/end of the subject).
fn matches_bug_keyword(subject: &str) -> bool {
    let lower = subject.to_lowercase();
    let bytes = lower.as_bytes();
    for kw in BUG_KEYWORDS {
        let kw_bytes = kw.as_bytes();
        let mut start = 0usize;
        while let Some(pos) = lower[start..].find(kw) {
            let abs = start + pos;
            let end = abs + kw_bytes.len();
            let before_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
            let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
            start = abs + 1;
        }
    }
    false
}
