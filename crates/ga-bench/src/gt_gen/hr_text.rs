//! Hr-text — `ga_risk` GT rule for M3 bench (5th tool).
//!
//! ## Anti-tautology policy
//! This rule does NOT import `ga_query::{dead_code, callers, rename_safety,
//! architecture, risk, minimal_context}` analysis types. Risky-file labels
//! come from raw git-log mining via `ga_query::blame::BlameMiner`
//! (substrate, not analysis surface — `ga_query::blame` exposes the same
//! mining helper to the production tool and the bench rule).
//!
//! See spec §C1 + AS for
//! the risk UC (added when spec is updated to pull risk into Phase 2).
//!
//! ## Algorithm
//! For each source file F in the fixture:
//!   subjects = blame.commit_subjects_in_window(F, anchor=HEAD, days=90)
//!   bug_count = subjects.filter(matches_bug_keyword).len()
//!   commit_count = subjects.len()
//!   risky = bug_count >= 1  // at least one bug-labelled commit in window
//! GT = list of (file, expected_risky, commit_count, bug_count).
//!
//! Anchored to the fixture HEAD's committer date so a fixture pinned to
//! a 2024 commit produces stable GT in 2026. Without anchoring, every run
//! mines an empty 90-day window and the GT is degenerate.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::walk::walk_repo;
use ga_query::blame::BUG_KEYWORDS;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

pub const RISK_WINDOW_DAYS: u32 = 90;

/// Cap files that get a per-file Hr-text entry. On django (50k files) the
/// previous per-file `git log` loop took 13+ minutes; cycle B switches to
/// a SINGLE repo-wide `git log --name-only` mining pass — bounded by
/// commit count, not file count — and only emits GT entries for files
/// that appear in those commits OR are otherwise asked about. Files with
/// zero commits in the window are not emitted (they would be vacuous
/// `expected_risky=false` rows that inflate the GT and bias scoring).
const MAX_COMMITS_TO_MINE: usize = 5000;

pub struct HrText;

impl Default for HrText {
    fn default() -> Self {
        Self
    }
}

impl GtRule for HrText {
    fn id(&self) -> &str {
        "Hr-text"
    }
    fn uc(&self) -> &str {
        "risk"
    }
    fn policy_bias(&self) -> &str {
        "Hr-text cycle B — risk labels mined from a SINGLE repo-wide \
         `git log --name-only` pass anchored to fixture HEAD's committer \
         date (not wall-clock). Drops the cycle-A per-file git_log loop \
         that took 13+ min on django (50k files × 2 subprocesses). Cost \
         now scales with commits-in-window, not files. Files with zero \
         commits in window are NOT emitted as GT entries (avoids a flood \
         of vacuous `expected_risky=false` rows inflating denominator). \
         Bug-keyword set: fix/bug/error/crash/regression (whole-word). \
         Shallow fixtures (httpx, radash — 1-commit) yield ≤1 commit \
         total → empty GT; `git fetch --unshallow` recommended before bench."
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let debug = std::env::var("GA_DEBUG_RISK_GT").is_ok();
        if debug {
            eprintln!("[hr_text] scan fixture={}", fixture_dir.display());
        }

        // Walk fixture to (1) confirm has source files, (2) build a
        // source-file allowlist so non-source files (.yml, .toml, .md,
        // tsconfig.json, LICENSE …) are excluded from GT.
        // Rationale: `ga_risk` semantics is "which SOURCE files are risky
        // to refactor". Non-source files appear in `git log --name-only`
        // (config, docs, test fixtures with bug commits) but engine's
        // impact pipeline can't analyze them — composite caps at the
        // test_gap=0.5 + bug carve-out so GT entries for non-source files
        // are systemic FN that don't reflect engine quality, just
        // GT-engine domain mismatch. 2026-05-02 audit: removing these
        // recovers nest from 8/10 FN to 4/10, regex/axum similar gains.
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;
        if debug {
            eprintln!(
                "[hr_text] walk_repo: {} source entries",
                report.entries.len()
            );
        }
        if report.entries.is_empty() {
            return Ok(Vec::new());
        }
        let source_files: std::collections::HashSet<String> = report
            .entries
            .iter()
            .map(|e| e.rel_path.to_string_lossy().to_string())
            .collect();

        // Resolve HEAD ref (commit SHA) once — anchor for time window.
        let anchor = resolve_head_sha(fixture_dir);
        if debug {
            eprintln!("[hr_text] resolve_head_sha: anchor='{anchor}'");
        }
        if anchor.is_empty() {
            return Ok(Vec::new());
        }

        // Mine commits in window with a single subprocess. `--name-only`
        // gives us subject + file list per commit; we accumulate per-file
        // counts in O(commits) instead of O(files × commits).
        let mined_all = mine_repo_in_window(fixture_dir, &anchor, RISK_WINDOW_DAYS);
        // Restrict to source files only (per source_files allowlist above).
        let mined: HashMap<String, FileStat> = mined_all
            .into_iter()
            .filter(|(f, _)| source_files.contains(f))
            .collect();
        if debug {
            let total_commits: u32 = mined.values().map(|s| s.commit_count).sum();
            let total_bugs: u32 = mined.values().map(|s| s.bug_count).sum();
            eprintln!(
                "[hr_text] mine_repo_in_window (source-only): {} files, {} total file-commit pairs, {} bug pairs",
                mined.len(),
                total_commits,
                total_bugs
            );
        }
        if mined.is_empty() {
            return Ok(Vec::new());
        }

        let mut out: Vec<GeneratedTask> = Vec::with_capacity(mined.len());
        for (file, stat) in mined {
            let expected_risky = stat.bug_count >= 1;
            let task_id = format!("hr-text::{file}");
            let query = json!({
                "file": file,
                "expected_risky": expected_risky,
                "commit_count": stat.commit_count,
                "bug_count": stat.bug_count,
                "anchor": anchor,
                "window_days": RISK_WINDOW_DAYS,
            });
            let expected = if expected_risky {
                vec![file.clone()]
            } else {
                vec![]
            };
            out.push(GeneratedTask {
                task_id,
                query,
                expected,
                rule: "Hr-text".to_string(),
                rationale: format!(
                    "{} commit(s) in last {RISK_WINDOW_DAYS} days before anchor {anchor}; \
                     {} matched bug-keyword set",
                    stat.commit_count, stat.bug_count
                ),
            });
        }
        Ok(out)
    }
}

#[derive(Default)]
struct FileStat {
    commit_count: u32,
    bug_count: u32,
}

/// Single-pass repo mining — `git log <anchor> --since=<anchor-N.days>
/// --until=<anchor> --pretty=format:<sentinel>%s --name-only --no-merges`.
/// Parses the stream into per-file `FileStat`. Cost scales with commit
/// count, not file count.
fn mine_repo_in_window(repo_root: &Path, anchor_ref: &str, days: u32) -> HashMap<String, FileStat> {
    let mut out: HashMap<String, FileStat> = HashMap::new();
    // Step 1 — anchor committer date as Unix timestamp (integer math
    // avoids the ISO 8601 date-arithmetic parsing trap that bit cycle B v1:
    // `git log --since="<iso> - 90 days"` doesn't parse, returning empty.
    let ts_out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show")
        .arg("-s")
        .arg("--format=%ct")
        .arg(anchor_ref)
        .output();
    let Ok(ts) = ts_out else {
        return out;
    };
    if !ts.status.success() {
        return out;
    }
    let anchor_unix: i64 = match String::from_utf8_lossy(&ts.stdout).trim().parse() {
        Ok(v) => v,
        Err(_) => return out,
    };
    let since_unix = anchor_unix - (days as i64) * 86400;

    let max_count = format!("--max-count={MAX_COMMITS_TO_MINE}");
    // git: --max-age = since (committer-time >= X);
    //      --min-age = until (committer-time <= X).
    // Window we want: committer-time in [since_unix, anchor_unix]
    // → --max-age=since_unix --min-age=anchor_unix.
    let log = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("log")
        .arg(anchor_ref)
        .arg(&max_count)
        .arg(format!("--max-age={since_unix}"))
        .arg(format!("--min-age={anchor_unix}"))
        .arg("--pretty=format:__HRTEXT__%s")
        .arg("--name-only")
        .arg("--no-merges")
        .output();
    let Ok(out_log) = log else {
        return out;
    };
    if !out_log.status.success() {
        return out;
    }

    let text = String::from_utf8_lossy(&out_log.stdout);
    let mut current_is_bug = false;
    for line in text.lines() {
        if let Some(subject) = line.strip_prefix("__HRTEXT__") {
            current_is_bug = matches_bug_keyword_local(subject);
        } else if !line.trim().is_empty() {
            let entry = out.entry(line.to_string()).or_default();
            entry.commit_count = entry.commit_count.saturating_add(1);
            if current_is_bug {
                entry.bug_count = entry.bug_count.saturating_add(1);
            }
        }
    }
    out
}

pub fn resolve_head_sha(repo_root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg("HEAD")
        .output();
    let Ok(o) = out else {
        return String::new();
    };
    if !o.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Mirrors `ga_query::blame::matches_bug_keyword` (private). Whole-word
/// case-insensitive match against `BUG_KEYWORDS`. Duplicating the helper
/// keeps the rule independent of the production analysis surface (the
/// keyword set itself is shared via the public `BUG_KEYWORDS` constant).
fn matches_bug_keyword_local(subject: &str) -> bool {
    let lower = subject.to_lowercase();
    let bytes = lower.as_bytes();
    for kw in BUG_KEYWORDS {
        let kw_lower = kw.to_lowercase();
        let kw_bytes = kw_lower.as_bytes();
        let mut start = 0;
        while let Some(pos) = lower[start..].find(&kw_lower) {
            let abs = start + pos;
            let before_ok =
                abs == 0 || (!bytes[abs - 1].is_ascii_alphanumeric() && bytes[abs - 1] != b'_');
            let after_idx = abs + kw_bytes.len();
            let after_ok = after_idx >= bytes.len()
                || (!bytes[after_idx].is_ascii_alphanumeric() && bytes[after_idx] != b'_');
            if before_ok && after_ok {
                return true;
            }
            start = abs + 1;
        }
    }
    false
}
