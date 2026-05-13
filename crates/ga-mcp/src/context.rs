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
use ga_index::Store;
use ga_parser::staleness::StalenessChecker;
use std::collections::HashMap;
use std::path::PathBuf;
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
}

impl McpContext {
    /// Construct with a StalenessChecker derived from the Store's
    /// `metadata.repo_root`. Default path for production callers and the
    /// existing test fixtures that didn't yet pass an explicit checker.
    pub fn new(store: Arc<Store>) -> Self {
        let repo_root = std::path::PathBuf::from(&store.metadata().repo_root);
        let staleness = Arc::new(StalenessChecker::new(repo_root));
        Self {
            store_cell: Arc::new(RwLock::new(Some(store))),
            staleness,
            reindex_locks: Arc::new(Mutex::new(HashMap::new())),
            last_reindex_at: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Test/integration construct with an injected StalenessChecker so
    /// callers can control the repo_root + TTL behavior independently.
    pub fn with_staleness(store: Arc<Store>, staleness: Arc<StalenessChecker>) -> Self {
        Self {
            store_cell: Arc::new(RwLock::new(Some(store))),
            staleness,
            reindex_locks: Arc::new(Mutex::new(HashMap::new())),
            last_reindex_at: Arc::new(Mutex::new(HashMap::new())),
        }
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
        let arc_store = guard.take().ok_or_else(|| Error::ReindexBuildFailed {
            reason: "store cell already empty before rebuild".to_string(),
        })?;
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
                    Err(Error::AlreadyReindexing {
                        hint: format!("peer process holds cache lock: {msg}"),
                    })
                } else {
                    // Leave cell None — next store() returns ReindexBuildFailed.
                    Err(Error::ReindexBuildFailed { reason: msg })
                }
            }
        }
    }

    /// PR6.1d AS-006 — record a successful reindex so the next call
    /// within `REINDEX_COOLDOWN_MS` short-circuits.
    pub fn record_reindex_success(&self, cache_dir: &std::path::Path) {
        let mut map = self
            .last_reindex_at
            .lock()
            .expect("last_reindex_at mutex");
        map.insert(cache_dir.to_path_buf(), Instant::now());
    }

    /// PR6.1d AS-006 — pre-check that returns `Err(AlreadyReindexing)`
    /// if a successful reindex happened within the cooldown window for
    /// this cache_dir. Returns `Ok(())` if no recent reindex (or the
    /// cooldown has expired).
    pub fn check_reindex_cooldown(&self, cache_dir: &std::path::Path) -> Result<()> {
        let map = self
            .last_reindex_at
            .lock()
            .expect("last_reindex_at mutex");
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
