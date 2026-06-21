//! Shared runtime context plumbed through MCP handlers. Holds the graph
//! [`Store`] so tool dispatchers can query the indexed graph without the
//! server loop passing it positionally.
//!
//! v1.5 PR6.1b — the inner store is wrapped in
//! `Arc<RwLock<Option<Arc<Store>>>>` so `ga_reindex` can take exclusive
//! ownership of the underlying `Store` (via `Arc::try_unwrap`), drop its
//! lbug handles + flock, run the close-rm-init rebuild, and reinstall a
//! fresh `Arc<Store>`. Read-side tool calls go through the `store()`
//! helper which clones a cheap `Arc<Store>` while the RwLock is held in
//! read mode — concurrent reads remain lock-free at the lbug layer.

use ga_core::{Error, Result};
use ga_index::cache::CacheLayout;
use ga_index::Store;
use ga_parser::staleness::StalenessChecker;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

/// PR6.1d AS-006 — post-success cooldown window. A `ga_reindex` that
/// fires within this window of a successful prior reindex short-circuits
/// with `Error::AlreadyReindexing` rather than running a redundant
/// rebuild. 200ms is short enough to be invisible to humans yet long
/// enough to absorb double-fires from FS watchers + hook installers
/// firing on the same edit.
const REINDEX_COOLDOWN_MS: u64 = 200;

#[derive(Clone)]
pub struct McpContext {
    /// Private — access via [`McpContext::store`]. `Option::None` is the
    /// transient sentinel between `ga_reindex`'s consume of the old Store
    /// and the install of the new one. A failed rebuild leaves `None` and
    /// subsequent `store()` calls return [`Error::ReindexBuildFailed`].
    store_cell: Arc<RwLock<Option<Arc<Store>>>>,
    /// v1.5 PR5 staleness gate (sub-spec staleness S-003).
    ///
    /// Pre-tool dispatch consults this checker to decide whether the live
    /// repo state has drifted from `metadata.indexed_root_hash`. 500ms TTL
    /// cache absorbs query bursts (AS-011). Constructed from the Store's
    /// `metadata.repo_root` at boot time.
    pub staleness: Arc<StalenessChecker>,
    /// v1.5 PR6 — per-repo `ga_reindex` serialization (sub-spec tool
    /// S-004 AS-009/010). Map key = canonical cache dir; value = Mutex
    /// guarding a single in-process reindex at a time. Cross-process
    /// coordination via flock (PR6.1 follow-up).
    pub reindex_locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>,
    /// PR6.1d AS-006 — per-repo last successful reindex timestamp.
    /// Subsequent rebuild attempts within `REINDEX_COOLDOWN_MS` of this
    /// instant short-circuit with `Error::AlreadyReindexing` rather than
    /// running a redundant rebuild.
    pub last_reindex_at: Arc<Mutex<HashMap<PathBuf, Instant>>>,
    /// Cache root (typically `~/.graphatlas`) captured at construction,
    /// INDEPENDENT of the store cell. Lets `rebuild_via` reopen the
    /// on-disk cache — to self-heal a busy/peer-lock failure, or to
    /// rebuild a previously-bricked (`None`) cell — without needing a
    /// live `Store` to read `repo_root`/`layout` from. The repo root is
    /// available via `self.staleness.repo_root()`.
    cache_root: Arc<PathBuf>,
}

/// Derive the cache root from a store's resolved layout: the layout dir is
/// `<cache_root>/<repo>-<hash>`, so its parent is the cache root. Falls back
/// to the layout dir itself if it has no parent (defensive; never expected).
fn cache_root_of(store: &Store) -> PathBuf {
    let dir = store.layout().dir();
    dir.parent().unwrap_or(dir).to_path_buf()
}

impl McpContext {
    /// Construct with a StalenessChecker derived from the Store's
    /// `metadata.repo_root`. Default path for production callers and the
    /// existing test fixtures that didn't yet pass an explicit checker.
    pub fn new(store: Arc<Store>) -> Self {
        let repo_root = std::path::PathBuf::from(&store.metadata().repo_root);
        let cache_root = Arc::new(cache_root_of(&store));
        let staleness = Arc::new(StalenessChecker::new(repo_root));
        Self {
            store_cell: Arc::new(RwLock::new(Some(store))),
            staleness,
            reindex_locks: Arc::new(Mutex::new(HashMap::new())),
            last_reindex_at: Arc::new(Mutex::new(HashMap::new())),
            cache_root,
        }
    }

    /// Test/integration construct with an injected StalenessChecker so
    /// callers can control the repo_root + TTL behavior independently.
    pub fn with_staleness(store: Arc<Store>, staleness: Arc<StalenessChecker>) -> Self {
        let cache_root = Arc::new(cache_root_of(&store));
        Self {
            store_cell: Arc::new(RwLock::new(Some(store))),
            staleness,
            reindex_locks: Arc::new(Mutex::new(HashMap::new())),
            last_reindex_at: Arc::new(Mutex::new(HashMap::new())),
            cache_root,
        }
    }

    /// Cache root (typically `~/.graphatlas`) captured at construction,
    /// independent of the store cell. Stays valid even when the cell is
    /// `None` after a failed rebuild.
    pub fn cache_root(&self) -> &Path {
        self.cache_root.as_ref()
    }

    /// Resolved cache directory for this repo (`<cache_root>/<repo>-<hash>`),
    /// computed from the cache root + the staleness checker's repo root.
    /// Available even when the store cell is `None` — used by `ga_reindex`
    /// so it can serialize + recover a bricked cell without a live `Store`.
    pub fn cache_dir(&self) -> PathBuf {
        CacheLayout::for_repo(self.cache_root.as_ref(), self.staleness.repo_root())
            .dir()
            .to_path_buf()
    }

    /// PR6.1b R1b-S001.AS-001 — fetch the current `Arc<Store>` for a tool
    /// dispatch. Cheap: takes the read side of the RwLock + clones the Arc.
    /// Panics if the store cell is in the `None` sentinel state — that
    /// only happens after a `ga_reindex` build failure, in which case the
    /// MCP is unhealthy and callers should observe `ReindexBuildFailed`
    /// via `try_store()` instead.
    pub fn store(&self) -> Arc<Store> {
        self.try_store()
            .expect("McpContext::store called while store cell is empty (post-reindex failure)")
    }

    /// Fallible variant of [`store`]. Returns [`Error::ReindexBuildFailed`]
    /// if the cell is `None` (a prior `ga_reindex` failed mid-rebuild).
    pub fn try_store(&self) -> Result<Arc<Store>> {
        let guard = self.store_cell.read().expect("store_cell rwlock poisoned");
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::ReindexBuildFailed {
                reason: "store cell empty after prior rebuild failure".to_string(),
            })
    }

    /// PR6.1b R1b-S002.AS-003 + STORE_BUSY — run a rebuild closure that
    /// consumes the current `Store` and produces a fresh one. Holds the
    /// write side of the RwLock for the duration so concurrent `store()`
    /// readers block until the swap completes.
    ///
    /// Returns [`Error::StoreBusy`] if other clones of the `Arc<Store>`
    /// are alive (refcount > 1) — caller should retry after in-flight
    /// tool calls drain. Returns [`Error::ReindexBuildFailed`] if the
    /// closure errors; on failure the cell is left in the `None` sentinel
    /// state and on-disk cache is empty (the closure already ran
    /// nuke + open against the old layout).
    pub fn rebuild_via<F>(&self, f: F) -> Result<Arc<Store>>
    where
        F: FnOnce(Store) -> Result<Store>,
    {
        let mut guard = self.store_cell.write().expect("store_cell rwlock poisoned");
        // Recover a previously-bricked (None) cell instead of refusing: open
        // a fresh handle from disk so the rebuild closure has a Store to work
        // with. This is what makes `ga_reindex` an actual recovery path for a
        // bricked server (docs/investigate/mcp-store-brick-hang-2026-06-21.md,
        // action 3) rather than itself failing on the empty cell.
        let arc_store = match guard.take() {
            Some(s) => s,
            None => Arc::new(
                self.open_from_disk()
                    .map_err(|e| Error::ReindexBuildFailed {
                        reason: format!("recover bricked cell: reopen cache failed: {e}"),
                    })?,
            ),
        };
        let store = match Arc::try_unwrap(arc_store) {
            Ok(s) => s,
            Err(still_shared) => {
                // Restore — refcount > 1 means in-flight tool calls.
                *guard = Some(still_shared);
                return Err(Error::StoreBusy);
            }
        };
        match f(store) {
            Ok(new_store) => {
                let new_arc = Arc::new(new_store);
                *guard = Some(Arc::clone(&new_arc));
                Ok(new_arc)
            }
            Err(e) => {
                // PR6.1d AS-011 — if the closure's error message carries a
                // busy/lock signature, classify as AlreadyReindexing
                // (-32014) so peer-process flock contention surfaces with
                // the right retry hint. Anything else stays
                // ReindexBuildFailed (-32012).
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                let busy_signature = lower.contains("busy")
                    || lower.contains("already in use")
                    || lower.contains("flock")
                    || (lower.contains("lock") && !lower.contains("deadlock"));
                if busy_signature {
                    // A peer holds the writer lock; OUR committed graph on
                    // disk is still valid. Reopen it read-only and restore
                    // the cell so this server keeps serving instead of
                    // bricking (action 1). Only leave the cell None if even
                    // the reopen fails — then `try_store()` surfaces the
                    // error gracefully and a later `ga_reindex` recovers.
                    match self.open_from_disk() {
                        Ok(reopened) => *guard = Some(Arc::new(reopened)),
                        Err(reopen_err) => tracing::warn!(
                            target: "ga_mcp::context",
                            "busy rebuild: reopen-to-self-heal failed, cell left empty: {reopen_err}"
                        ),
                    }
                    Err(Error::AlreadyReindexing {
                        hint: format!("peer process holds cache lock: {msg}"),
                    })
                } else {
                    // Genuine build failure: the on-disk cache was nuked by
                    // the failed rebuild, so there is nothing valid to reopen.
                    // Leave cell None — next try_store() returns
                    // ReindexBuildFailed; ga_reindex can rebuild from scratch.
                    Err(Error::ReindexBuildFailed { reason: msg })
                }
            }
        }
    }

    /// Open the on-disk cache for this repo, sealed read-only for serving.
    /// Used by `rebuild_via` to self-heal a busy failure or recover a
    /// bricked cell. Caller holds the store-cell write lock; this touches
    /// only the filesystem + lbug, so there is no lock re-entrancy.
    fn open_from_disk(&self) -> Result<Store> {
        let repo_root = self.staleness.repo_root().to_path_buf();
        let mut store = Store::open_with_root(self.cache_root.as_ref(), &repo_root)?;
        // Best-effort seal: a fresh writer handle is fine to serve reads
        // from too, so a seal failure is not fatal to recovery.
        let _ = store.seal_for_serving();
        Ok(store)
    }

    /// v1.5 PR6.1 (multi-mcp) H-4 — try to reopen the Store's lbug handle
    /// if a peer writer bumped the on-disk `graph_generation` since this
    /// process last opened. Best-effort: takes the write lock briefly;
    /// if the Arc refcount is > 1 (concurrent tool call in flight) OR
    /// the cell is empty (mid-rebuild), skip silently and return Ok(false).
    ///
    /// Called from `dispatch_tool_call_with_ctx` at handler entry so
    /// long-running readers bound their inode pinning to a single
    /// generation skew across tool calls (per spec S-002 AS-007
    /// invariant). Inexpensive in the common case — `reopen_if_stale`
    /// reads only `metadata.json` (~200B) and short-circuits when
    /// generation matches.
    pub fn refresh_if_stale(&self) -> Result<bool> {
        let mut guard = self.store_cell.write().expect("store_cell rwlock poisoned");
        let Some(arc_store) = guard.take() else {
            return Ok(false);
        };
        let mut store = match Arc::try_unwrap(arc_store) {
            Ok(s) => s,
            Err(still_shared) => {
                *guard = Some(still_shared);
                return Ok(false);
            }
        };
        let reopened = store.reopen_if_stale().unwrap_or_else(|e| {
            tracing::warn!(
                target: "ga_mcp::context",
                "reopen_if_stale failed (continuing with stale view): {e}"
            );
            false
        });
        *guard = Some(Arc::new(store));
        Ok(reopened)
    }

    /// PR6.1d AS-006 — record a successful reindex so the next call
    /// within `REINDEX_COOLDOWN_MS` short-circuits.
    pub fn record_reindex_success(&self, cache_dir: &std::path::Path) {
        let mut map = self.last_reindex_at.lock().expect("last_reindex_at mutex");
        map.insert(cache_dir.to_path_buf(), Instant::now());
    }

    /// PR6.1d AS-006 — pre-check that returns `Err(AlreadyReindexing)`
    /// if a successful reindex happened within the cooldown window for
    /// this cache_dir. Returns `Ok(())` if no recent reindex (or the
    /// cooldown has expired).
    pub fn check_reindex_cooldown(&self, cache_dir: &std::path::Path) -> Result<()> {
        let map = self.last_reindex_at.lock().expect("last_reindex_at mutex");
        if let Some(last) = map.get(cache_dir) {
            let elapsed_ms = last.elapsed().as_millis() as u64;
            if elapsed_ms < REINDEX_COOLDOWN_MS {
                return Err(ga_core::Error::AlreadyReindexing {
                    hint: format!(
                        "post-success cooldown active ({elapsed_ms}ms ago, {REINDEX_COOLDOWN_MS}ms window)"
                    ),
                });
            }
        }
        Ok(())
    }

    /// v1.5 PR6 AS-009/010 — fetch (or lazily create) the per-repo
    /// reindex serialization lock for the cache dir.
    pub fn reindex_lock_for(&self, cache_dir: &std::path::Path) -> Arc<Mutex<()>> {
        let mut map = self.reindex_locks.lock().expect("reindex_locks mutex");
        map.entry(cache_dir.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}
