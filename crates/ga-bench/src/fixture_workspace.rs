//! Per-gate fixture scratch — keep canonical submodule clones immutable.
//!
//! M2 and M3 benches both checkout per-task `base_commit` to drive their
//! indexer over the file-system-shaped working tree. If both gates mutate
//! `benches/fixtures/<repo>` directly, running them in the same `cargo test`
//! invocation (or even in two sessions on one machine) leaves the submodule
//! HEAD drifted from the super-project pin and corrupts each other's runs.
//!
//! Fix: each gate gets its own scratch clone under
//! `<cache_root>/fixtures-<gate>/<repo>`. Cloned with `git clone --local` so
//! object files are hardlinked from the canonical submodule — fast, near-zero
//! disk overhead. Mutation lives entirely in the scratch; the submodule never
//! gets touched.
//!
//! Cache invalidation: if the canonical submodule pointer moves (super-project
//! advanced), the scratch's stale objects are still valid (--local copied
//! everything reachable at clone time). New commits the scratch needs will
//! fail to checkout and the caller skips the task — caller can `rm -rf` the
//! scratch dir to force a fresh clone.

use crate::BenchError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// v1.2-php S-002 AS-020 — assert the fixture workspace is in a clean state
/// before mining or scoring. Catches the `project_m3_submodule_drift` failure
/// mode where an M3 HmcGitmine run leaves the scratch dirty and the next M2
/// run silently inherits the dirty tree, producing biased numbers.
///
/// Returns `Err(BenchError::Other("FixtureCorrupted: ..."))` if:
/// - `git status --porcelain` produces non-empty output (uncommitted changes)
/// - `path` is not a git repo
///
/// Returns `Ok(())` if the workspace tree matches HEAD exactly. Callers
/// should invoke this in M2 runner's `run()` entry point per-fixture, BEFORE
/// the first per-task `base_commit` checkout.
pub fn assert_workspace_clean(path: &Path) -> Result<(), BenchError> {
    if !path.join(".git").exists() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "FixtureCorrupted: not a git repo: {}",
            path.display()
        )));
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| {
            BenchError::Other(anyhow::anyhow!(
                "FixtureCorrupted: git status -C {}: {e}",
                path.display()
            ))
        })?;
    if !out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "FixtureCorrupted: git status -C {} exited non-zero: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    if !out.stdout.is_empty() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "FixtureCorrupted: {} has uncommitted changes — likely M3-drift contamination per project_m3_submodule_drift memory. Run `git -C {} reset --hard` (or re-clone scratch) before continuing. \n--- git status --porcelain ---\n{}",
            path.display(),
            path.display(),
            String::from_utf8_lossy(&out.stdout)
        )));
    }
    Ok(())
}

/// Resolve (or create on first use) a per-gate scratch clone of `source`.
/// `gate` is a short label (e.g. `"m2"`, `"m3"`). `source` should be the
/// canonical submodule directory (`benches/fixtures/<repo>`). Returns the
/// scratch path; callers checkout / index / mutate that path freely.
pub fn ensure_gate_scratch(
    gate: &str,
    source: &Path,
    cache_root: &Path,
) -> Result<PathBuf, BenchError> {
    let repo_name = source.file_name().ok_or_else(|| {
        BenchError::Other(anyhow::anyhow!(
            "fixture path missing file name: {}",
            source.display()
        ))
    })?;
    let scratch = cache_root.join(format!("fixtures-{gate}")).join(repo_name);

    if scratch.join(".git").exists() {
        return Ok(scratch);
    }

    if let Some(parent) = scratch.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            BenchError::Other(anyhow::anyhow!(
                "create scratch parent {}: {e}",
                parent.display()
            ))
        })?;
    }

    // `--local` hardlinks pack/object files from the canonical submodule —
    // fast, near-zero disk cost. Standard git operations only add new packs;
    // they never modify existing files in-place, so the hardlinks are safe
    // even when the scratch is checked out at a different commit.
    let out = Command::new("git")
        .arg("clone")
        .arg("--local")
        .arg(source)
        .arg(&scratch)
        .output()
        .map_err(|e| BenchError::Other(anyhow::anyhow!("git clone: {e}")))?;
    if !out.status.success() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "git clone --local {} -> {} failed: {}",
            source.display(),
            scratch.display(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }

    Ok(scratch)
}
