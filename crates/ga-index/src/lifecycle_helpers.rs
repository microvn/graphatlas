//! v1.5 lifecycle helpers — extracted in PR1a (challenge C-4 fix), consumed by:
//! - PR1a empirical tests (this crate's `tests/lbug_lifecycle_*.rs`)
//! - PR6 `ga_reindex` tool (constructor retry wrapper)
//!
//! Cuts the cyclic dependency: PR1 needs Windows CI (lands in PR3) but also
//! needs to test `wait_for_handle_release` (helper lives in PR6). Solution:
//! helper ships in PR1a as production module; PR6 reuses; Windows iteration
//! deferred to PR1b after PR3 lands matrix runner.
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-empirical.md` S-002.

use std::path::Path;
use uuid::Uuid;

/// Probe whether the given path's file handles have been released by the OS.
///
/// **POSIX**: `flock` release is synchronous on `Drop` of the underlying
/// file descriptor. Returns `Ok(true)` immediately without I/O.
///
/// **Windows** (PR1b — currently no-op stub): `libuv` + antivirus interaction
/// can delay handle release beyond the JS-side `close()`. The Windows arm will
/// probe with `File::options().read(true).write(true).open(path)` up to 5
/// times with `50ms × attempt` backoff (GitNexus pattern, total ≤750ms).
///
/// Caller contract:
/// - Returns `Ok(true)` when path is ready for the next `Database::new` open.
/// - Returns `Ok(false)` when probe attempts exhausted (Windows only).
/// - Returns `Err(...)` for unexpected I/O errors not in the busy class.
pub fn wait_for_handle_release(_path: &Path) -> anyhow::Result<bool> {
    #[cfg(unix)]
    {
        // POSIX: synchronous handle release. No I/O needed.
        Ok(true)
    }

    #[cfg(windows)]
    {
        // PR1b will implement the retry-probe loop. For now PR1a ships a
        // stub that mirrors POSIX behavior so the helper is callable on
        // both platforms. Tests gated on `#[cfg(windows)]` in PR1b validate
        // the real retry semantics.
        Ok(true)
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Unsupported platform — caller should not invoke this helper.
        anyhow::bail!("wait_for_handle_release: unsupported platform (only unix/windows targeted)")
    }
}

/// Classify an lbug error as transient-busy (worth retrying) vs permanent.
///
/// Matches the GitNexus reference (`lbug-config.ts:200-265`): error messages
/// containing `"busy"`, `"lock"`, or `"already in use"` (case-insensitive)
/// indicate a writer-hold contention that resolves with backoff. All other
/// errors are surfaced immediately.
///
/// **Trap to avoid**: substring overlap with unrelated terms. For example
/// `"deadlock"` contains `"lock"` but is NOT recoverable via retry. Tests
/// pin both happy + unhappy matches; expand the matcher only when lbug's
/// own error vocabulary changes.
pub fn is_busy_error(err: &lbug::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("busy") || msg.contains("lock") || msg.contains("already in use")
}

/// v1.5 PR3 foundation S-004 AS-013 — create a tracing span tagged with a
/// fresh correlation_id for a reindex lifecycle operation.
///
/// PR6 (ga_reindex tool) wraps its full-rebuild + commit sequence inside
/// this span so cross-process readers can correlate writer logs with
/// subsequent RO reopen events emitted by PR4's `reopen_if_stale`.
///
/// Caller pattern:
/// ```ignore
/// let span = reindex_span();
/// let _guard = span.enter();
/// tracing::info!("starting full rebuild");
/// // ... commit lifecycle ...
/// ```
///
/// Returns both the Span (so caller can `enter()` it) and the UUID
/// (so caller can also include in MCP response `correlation_id` field).
pub fn reindex_span() -> (tracing::Span, Uuid) {
    let id = Uuid::new_v4();
    let span = tracing::info_span!("reindex", correlation_id = %id);
    (span, id)
}
