//! Hh-gitmine — `ga_hubs` GT rule.
//!
//! ## Anti-tautology policy
//! This rule does NOT call `ga_query::hubs` or any other engine analysis
//! type. The expected hub set is derived from `git log --name-only` —
//! files that received the most commits in the look-back window are
//! "hubs in practice" (the symbols teams keep editing). Engine output
//! is then scored by Spearman correlation against this oracle.
//!
//! Trade-off: file-level signal, not symbol-level. The ga_hubs UC
//! returns symbol entries; we project those down to a file rank and
//! correlate. A symbol-level oracle would need `git blame` per range
//! across the look-back commits — heavy and noisy.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Look-back window in months. 12 = balance "fresh enough to reflect
/// current architecture" against "long enough to drown out short bursts".
const SINCE_MONTHS: u32 = 12;

/// Top-N file list emitted as the GT rank. Capped well above ga_hubs's
/// natural top_n so Spearman has headroom for tail comparisons.
const GT_TOP_N: usize = 50;

pub struct HhGitmine {
    pub since_months: u32,
    pub top_n: usize,
}

impl Default for HhGitmine {
    fn default() -> Self {
        Self {
            since_months: SINCE_MONTHS,
            top_n: GT_TOP_N,
        }
    }
}

impl GtRule for HhGitmine {
    fn id(&self) -> &str {
        "Hh-gitmine"
    }

    fn uc(&self) -> &str {
        "hubs"
    }

    fn policy_bias(&self) -> &str {
        "Hh-gitmine — file-level oracle. Counts non-binary file touches in \
         `git log --name-only` over the last 12 months BEFORE HEAD's \
         committer timestamp (NOT relative to wall-clock now — fixtures \
         are pinned at base_commit per CLAUDE.md, so a wall-clock window \
         would silently exclude older fixtures). Ranks files by touch \
         frequency. Engine output (symbol-level hubs) is projected to \
         its file set and scored by Spearman rank correlation. \
         Bias 1: file-granularity — a file with one giant hub function \
         ties with a file holding 20 small symbols (rank is per-file, \
         not per-symbol). \
         Bias 2: pre-merge churn (rebases, squashed PRs) doesn't always \
         reflect long-term architectural pressure — fixtures with squashy \
         histories under-represent hubs. \
         Bias 3: HEAD-anchored window means very-young fixtures (HEAD < \
         12 months after first commit) have a smaller effective window."
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let ranked = mine_file_churn(fixture_dir, self.since_months)?;
        // Single per-fixture task carrying the ranked file list as
        // `expected`. Position in the vec = rank (index 0 is most-churned).
        let top: Vec<String> = ranked
            .into_iter()
            .map(|(path, _)| path)
            .take(self.top_n)
            .collect();
        if top.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![GeneratedTask {
            task_id: format!(
                "Hh-gitmine-{}",
                fixture_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("repo")
            ),
            query: json!({
                "since_months": self.since_months,
                "top_n": self.top_n,
            }),
            expected: top,
            rule: "Hh-gitmine".to_string(),
            rationale: format!(
                "Top files by `git log --name-only` touch frequency in the \
                 {}-month window before HEAD; cf. policy_bias",
                self.since_months
            ),
        }])
    }
}

/// Mine `(rel_path, touch_count)` from `git log --name-only`, anchored to
/// HEAD's committer timestamp (NOT now). Window = last `since_months`
/// before HEAD. Without HEAD-anchoring, fixtures pinned at historical
/// `base_commit`s (per CLAUDE.md "M2 + M3 runners checkout per-task
/// base_commit") return 0 commits when run later than the window from
/// today — breaking the gate silently.
///
/// Pattern matches `gt_gen::hr_text::mine_repo_in_window` (M3 risk UC):
/// fetch HEAD's `%ct`, derive `since_unix = head_ct - days * 86400`,
/// pass `--max-age` / `--min-age` (committer-time bounds, integer math
/// to avoid the ISO-arithmetic trap noted in hr_text:188).
pub fn mine_file_churn(
    repo_root: &Path,
    since_months: u32,
) -> Result<Vec<(String, u32)>, BenchError> {
    // Step 1: HEAD committer timestamp — anchor for the window.
    let ts_out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["show", "-s", "--format=%ct", "HEAD"])
        .output()
        .map_err(|e| {
            BenchError::Other(anyhow::anyhow!(
                "git show HEAD subprocess in {}: {e}",
                repo_root.display()
            ))
        })?;
    if !ts_out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "git show -s --format=%ct HEAD failed in {}: status={}",
            repo_root.display(),
            ts_out.status
        )));
    }
    let head_ts: i64 = String::from_utf8_lossy(&ts_out.stdout)
        .trim()
        .parse()
        .map_err(|e| {
            BenchError::Other(anyhow::anyhow!(
                "parse HEAD %ct in {}: {e}",
                repo_root.display()
            ))
        })?;
    // 30 d/mo simple back-calc; a one-day drift either way doesn't move
    // a top-50 file rank in practice.
    let since_ts = head_ts
        .saturating_sub((since_months as i64) * 30 * 86_400)
        .max(0);

    // Step 2: log within window. --max-age = committer-time >= since;
    // --min-age = committer-time <= anchor (HEAD).
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "log",
            "HEAD",
            &format!("--max-age={since_ts}"),
            &format!("--min-age={head_ts}"),
            "--name-only",
            "--pretty=tformat:",
            "--no-renames",
        ])
        .output()
        .map_err(|e| {
            BenchError::Other(anyhow::anyhow!(
                "git log subprocess in {}: {e}",
                repo_root.display()
            ))
        })?;
    if !out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "git log failed in {}: status={}",
            repo_root.display(),
            out.status
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if !is_source_path(line) {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }

    let mut ranked: Vec<(String, u32)> = counts.into_iter().collect();
    // Sort by count DESC, then path ASC for reproducibility on ties.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(ranked)
}

/// Conservative filter — only excludes files we're confident don't carry
/// architecturally meaningful symbols. Tweak if a fixture surfaces a
/// false-negative source path.
fn is_source_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    // Exclude lock files / package manifests by name.
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    if matches!(
        basename,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "cargo.lock"
            | "poetry.lock"
            | "pipfile.lock"
            | "gemfile.lock"
            | "go.sum"
            | "composer.lock"
            | "podfile.lock"
            | ".gitignore"
            | ".gitmodules"
            | ".gitattributes"
            | "license"
            | "license.md"
            | "license.txt"
            | "readme"
            | "readme.md"
            | "readme.rst"
            | "changelog"
            | "changelog.md"
    ) {
        return false;
    }
    // Exclude common non-source extensions.
    if let Some(dot) = lower.rfind('.') {
        let ext = &lower[dot + 1..];
        if matches!(
            ext,
            "md" | "rst"
                | "txt"
                | "json"
                | "lock"
                | "toml"
                | "yaml"
                | "yml"
                | "ini"
                | "cfg"
                | "conf"
                | "csv"
                | "tsv"
                | "svg"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "pdf"
                | "min"
                | "map"
        ) {
            return false;
        }
    }
    // Exclude common build/output dirs.
    let prefix_excludes = [
        "node_modules/",
        "vendor/",
        "third_party/",
        "dist/",
        "build/",
        "target/",
        ".git/",
        "docs/",
    ];
    for p in prefix_excludes {
        if path.starts_with(p) {
            return false;
        }
    }
    true
}
