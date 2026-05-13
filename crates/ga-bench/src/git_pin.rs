//! Shared git pinning primitive for M2 + M3 benches.
//!
//! Per user direction (2026-04-28) M1/M2/M3 should stay independent in
//! their **GT semantics** (own loaders, own rules), but it's wasteful to
//! re-implement the pinning shell-out twice. Both `m2_runner.rs` and
//! `m3_minimal_context.rs` consume these helpers — that's an allowed
//! shared primitive (Codex round 3 review: "infra coupling, not GT
//! semantics").

use crate::BenchError;
use std::path::Path;
use std::process::Command;

/// `git rev-parse HEAD` in `repo_dir`. Returns the current HEAD sha so
/// callers can restore it after they're done iterating tasks.
pub fn git_head(repo_dir: &Path) -> Result<String, BenchError> {
    let out = Command::new("git")
        .args(["-C", &repo_dir.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .map_err(|e| BenchError::Other(anyhow::anyhow!("git rev-parse: {e}")))?;
    if !out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git checkout -q <sha>` in `repo_dir`. Errors carry the git stderr so
/// callers can decide between "skip task + score 0" (M2 + M3 default) and
/// "abort the whole run".
pub fn git_checkout(repo_dir: &Path, sha: &str) -> Result<(), BenchError> {
    let out = Command::new("git")
        .args(["-C", &repo_dir.display().to_string(), "checkout", "-q", sha])
        .output()
        .map_err(|e| BenchError::Other(anyhow::anyhow!("git checkout: {e}")))?;
    if !out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "git checkout {sha} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}
