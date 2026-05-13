//! Top-level Store wiring: cache layout + metadata + PID lock + lbug Database.
//!
//! `Store` owns the `lbug::Database`. Callers get fresh `lbug::Connection`s via
//! [`Store::connection`] — Connection borrows from Database so it stays
//! per-scope (pattern validated in the ADR-001 spike).

use crate::cache::CacheLayout;
use crate::generation::bump_graph_generation;
use crate::lock::LockFile;
use crate::metadata::{Metadata, SchemaDecision};
use crate::nuke::{nuke_cache_files, placeholder_metadata};
use crate::root_hash::populate_root_hash;
use crate::SCHEMA_VERSION;
use ga_core::{Error, Result};
use std::path::{Path, PathBuf};

// v1.5 PR6.1a tech-debt split — log line helpers moved to
// `crate::log_lines`. Re-exported here so legacy callers using
// `ga_index::store::rebuild_log_line_*` keep working without churn.
pub use crate::log_lines::{
    rebuild_log_line_crash_recovery, rebuild_log_line_schema_mismatch,
    rebuild_log_line_schema_upgrade,
};

#[derive(Debug)]
pub enum OpenOutcome {
    /// No cache was present — a fresh build session started.
    FreshBuild,
    /// Cache on disk matched the binary's schema and was `complete` — reusable.
    Resumed,
    /// On-disk schema_version differed — old cache wiped, fresh build started.
    RebuildSchemaMismatch { cache: u32, binary: u32 },
    /// On-disk state was `building` (previous run crashed) — wiped + rebuilding.
    RebuildCrashRecovery { stale_generation: String },
    /// Another instance is the writer for this repo; we attached as a read-only
    /// reader. No mutations possible from this Store. Caller (typically MCP)
    /// should serve query traffic and skip indexing.
    AttachedReadOnly { writer_generation: String },
}

pub struct Store {
    // v1.5 PR2 audit bug #2 (foundation S-001 AS-002): `db` is `Option` so
    // `seal_for_serving` can `take()` and drop the RW handle BEFORE opening
    // the RO one. lbug 0.16.1 docs forbid having a RW Database and a RO
    // Database alive concurrently in the same process — even briefly.
    db: Option<lbug::Database>,
    metadata: Metadata,
    // Held for the lifetime of the Store so cross-process peers see contention.
    // Exclusive = this process is the writer. Shared = read-only attached reader.
    _lock: LockFile,
    outcome: OpenOutcome,
    layout: CacheLayout,
    committed: bool,
    read_only: bool,
}

impl Store {
    /// Open the cache at the default root (`~/.graphatlas`) for the given repo.
    pub fn open(repo_root: &Path) -> Result<Self> {
        let cache_root = default_cache_root()?;
        Self::open_with_root(&cache_root, repo_root)
    }

    /// Open with an explicit cache root — primarily for tests and `graphatlas list`.
    pub fn open_with_root(cache_root: &Path, repo_root: &Path) -> Result<Self> {
        Self::open_with_root_and_schema(cache_root, repo_root, SCHEMA_VERSION)
    }

    /// Lower-level variant that lets tests pin the binary schema version.
    pub fn open_with_root_and_schema(
        cache_root: &Path,
        repo_root: &Path,
        binary_schema: u32,
    ) -> Result<Self> {
        // Foundation-C8: reject sensitive override targets (e.g. ~/.ssh, /etc)
        // and pre-existing dirs with mode > 0700 before we write anything.
        crate::cache::validate_cache_dir_override(cache_root)?;
        // Ensure the cache root itself is 0700 — `std::fs::create_dir_all`
        // uses umask so a freshly-created root is typically 0755 on macOS
        // (umask 022). Clamp now so the next open doesn't trip the mode check.
        crate::cache::ensure_cache_root(cache_root)?;

        let layout = CacheLayout::for_repo(cache_root, repo_root);
        layout.ensure_dir()?;

        // Acquire the lock FIRST — before any mutation (nuke, begin_indexing).
        // v1 acquired after both, which let two simultaneous starters race
        // through `nuke_cache_files` + `begin_indexing_with_schema` before
        // either grabbed the lock. Locking on a probe `index_generation`
        // (rewritten on success path below) is fine: the kernel flock is
        // identity-free, and the sidecar JSON is only diagnostic.
        let lock = match LockFile::try_acquire_exclusive(&layout, "probe") {
            Ok(l) => l,
            Err(_held) => {
                // Writer is busy. Try to attach as a read-only reader so a
                // second `graphatlas mcp` (or any query-only caller) can still
                // serve traffic against the cache the writer has already
                // committed — instead of failing the whole process.
                return Self::open_read_only(cache_root, repo_root, binary_schema);
            }
        };

        let decision = Metadata::cold_load(&layout, binary_schema)?;
        let outcome = match &decision {
            SchemaDecision::NoCache => OpenOutcome::FreshBuild,
            SchemaDecision::Match(_) => OpenOutcome::Resumed,
            SchemaDecision::Mismatch { cache, binary } => OpenOutcome::RebuildSchemaMismatch {
                cache: *cache,
                binary: *binary,
            },
            SchemaDecision::CrashedBuilding { generation } => OpenOutcome::RebuildCrashRecovery {
                stale_generation: generation.clone(),
            },
        };

        // On any rebuild path, nuke prior cache files so lbug opens clean (spike
        // finding: lbug stores graph.db as a FILE on macOS; remove_dir_all on it
        // fails silently → need nuke-both-forms helper). Also emit the
        // spec-literal log line on stderr (AS-008, AS-027, AS-025).
        match &decision {
            SchemaDecision::NoCache | SchemaDecision::Match(_) => {}
            SchemaDecision::Mismatch { cache, binary } => {
                eprintln!("{}", rebuild_log_line_schema_mismatch(*cache, *binary));
                eprintln!("{}", rebuild_log_line_schema_upgrade(*binary));
                nuke_cache_files(&layout);
            }
            SchemaDecision::CrashedBuilding { .. } => {
                eprintln!("{}", rebuild_log_line_crash_recovery());
                nuke_cache_files(&layout);
            }
        }

        let repo_root_str = repo_root.to_string_lossy().to_string();
        let metadata = match decision {
            SchemaDecision::Match(m) => m,
            _ => Metadata::begin_indexing_with_schema(&layout, &repo_root_str, binary_schema)?,
        };

        // AS-029: if graph.db already exists (Resumed path), refuse if perms are
        // not 0600 before we even let lbug open it.
        let db_path = layout.graph_db();
        if db_path.exists() {
            crate::cache::verify_file_perms(&db_path)?;
        }

        let db = lbug::Database::new(&db_path, lbug::SystemConfig::default())
            .map_err(|e| Error::Database(format!("open graph.db failed: {e}")))?;

        // AS-029: lbug creates graph.db with the process umask. Clamp to 0600.
        if db_path.exists() {
            crate::cache::chmod_0600(&db_path)?;
        }

        // Apply graph schema. v1.3-Tools-C9 split:
        //   1. BASE_DDL_STATEMENTS — idempotent CREATE … IF NOT EXISTS,
        //      safe to replay every open. Holds the full v4 schema.
        //   2. MIGRATION_STATEMENTS — ALTER ADD COLUMN forms. EMPTY in v1.3
        //      PR1 because the cold_load mismatch path nukes + rebuilds for
        //      cache-version bumps; CREATE-with-DEFAULT covers fresh builds.
        //      Reserved for future schema bumps that need true incremental
        //      migration on already-deployed caches.
        {
            let conn = lbug::Connection::new(&db)
                .map_err(|e| Error::Database(format!("schema conn: {e}")))?;
            for stmt in crate::schema::BASE_DDL_STATEMENTS {
                conn.query(stmt)
                    .map_err(|e| Error::Database(format!("schema DDL failed ({stmt}): {e}")))?;
            }
            // MIGRATION_STATEMENTS — only run when not Resumed (i.e., on
            // fresh build or post-nuke rebuild). lbug ALTER ADD COLUMN has
            // no idempotent form, so replay on Resumed cache would error.
            let needs_migration = !matches!(outcome, OpenOutcome::Resumed);
            if needs_migration {
                for stmt in crate::schema::MIGRATION_STATEMENTS {
                    conn.query(stmt).map_err(|e| {
                        Error::Database(format!("schema MIGRATION failed ({stmt}): {e}"))
                    })?;
                }
            }
        }

        // v1.5 PR2 audit bug #4 (foundation S-001 AS-004): the lock was
        // acquired with the literal "probe" placeholder so it could fire
        // BEFORE metadata was generated. Now that `begin_indexing_with_schema`
        // has minted the real UUID (or we have a Resumed metadata), rewrite
        // the lock.pid sidecar in place so peers see the real generation.
        let _ = lock.update_sidecar(&metadata.index_generation);

        Ok(Self {
            db: Some(db),
            metadata,
            _lock: lock,
            outcome,
            layout,
            committed: false,
            read_only: false,
        })
    }

    /// Read-only attach: another process holds the exclusive writer lock for
    /// this repo. We acquire a shared lock and open lbug in read-only mode so
    /// query traffic still works. No DDL, no nuke, no metadata mutation.
    ///
    /// Fails if (a) we cannot get even a shared lock (transient — writer is
    /// mid-flock-conversion) or (b) on-disk metadata says `Building`/missing
    /// (no committed cache to read from). The caller should treat (b) as
    /// "indexing in progress, retry shortly" rather than a hard error.
    fn open_read_only(cache_root: &Path, repo_root: &Path, binary_schema: u32) -> Result<Self> {
        let layout = CacheLayout::for_repo(cache_root, repo_root);
        layout.ensure_dir()?;

        let lock = LockFile::try_acquire_shared(&layout, "reader").map_err(|e| {
            Error::Other(anyhow::anyhow!(
                "another graphatlas instance is indexing this repo and read-only \
                 attach failed: {e}"
            ))
        })?;

        // Only attach if the cache is committed. A `Building` state means the
        // writer's lbug DB may be mid-write — opening it for read could see
        // partial state. Refuse with a retryable error.
        let decision = Metadata::cold_load(&layout, binary_schema)?;
        let metadata = match decision {
            SchemaDecision::Match(m) => m,
            SchemaDecision::NoCache | SchemaDecision::CrashedBuilding { .. } => {
                return Err(Error::Other(anyhow::anyhow!(
                    "graphatlas: another instance is indexing this repo; no committed \
                     cache yet — retry once the initial build completes"
                )));
            }
            SchemaDecision::Mismatch { cache, binary } => {
                return Err(Error::Other(anyhow::anyhow!(
                    "graphatlas: cache schema v{cache} does not match binary v{binary}; \
                     a writer is currently rebuilding — retry shortly"
                )));
            }
        };

        let writer_generation = metadata.index_generation.clone();
        let db_path = layout.graph_db();
        if !db_path.exists() {
            return Err(Error::Other(anyhow::anyhow!(
                "graphatlas: metadata reports complete but graph.db missing; \
                 writer may have crashed mid-commit — retry shortly"
            )));
        }
        crate::cache::verify_file_perms(&db_path)?;

        let db = lbug::Database::new(&db_path, lbug::SystemConfig::default().read_only(true))
            .map_err(|e| Error::Database(format!("open graph.db read-only failed: {e}")))?;

        Ok(Self {
            db: Some(db),
            metadata,
            _lock: lock,
            outcome: OpenOutcome::AttachedReadOnly { writer_generation },
            layout,
            committed: true, // already committed by the writer; we never mutate
            read_only: true,
        })
    }

    /// True when this Store was attached as a read-only reader (another
    /// process holds the writer lock).
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Fresh Connection borrowing from the owned Database. Per-scope per AS-009.
    ///
    /// v1.5 PR2: returns an explicit Err if `seal_for_serving` is in the
    /// process of swapping handles (db = None transiently). Callers should
    /// retry after the seal completes, or treat as Err in tests.
    pub fn connection(&self) -> Result<lbug::Connection<'_>> {
        let db = self.db.as_ref().ok_or_else(|| {
            Error::Database(
                "Store has no active lbug::Database (mid-seal transition or poisoned)"
                    .to_string(),
            )
        })?;
        lbug::Connection::new(db).map_err(|e| Error::Database(format!("connection: {e}")))
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn outcome(&self) -> &OpenOutcome {
        &self.outcome
    }

    pub fn layout(&self) -> &CacheLayout {
        &self.layout
    }

    /// Atomically transition `building → complete`. Called after indexing succeeds.
    ///
    /// v1.5 PR2 audit bug #1 (foundation S-001 AS-001): populates
    /// `indexed_root_hash` with the Merkle root of the repo before write.
    /// v1.5 PR2 audit bug #3 (foundation S-001 AS-003): seal errors propagate
    /// via `?` instead of being silently swallowed.
    pub fn commit(mut self) -> Result<()> {
        if self.read_only {
            return Err(Error::Other(anyhow::anyhow!(
                "Store is read-only (attached as reader to another writer); commit refused"
            )));
        }
        populate_root_hash(&mut self.metadata)?;
        // v1.5 PR4 staleness Phase B: bump generation in lbug `GraphMeta`
        // table (authoritative) then mirror to metadata.json. Write the
        // lbug row BEFORE the metadata.json transition so the persisted
        // mirror cannot ever exceed the lbug source of truth.
        bump_graph_generation(&mut self.metadata, self.db.as_ref())?;
        let md = std::mem::replace(&mut self.metadata, placeholder_metadata());
        let md = md.commit(&self.layout)?;
        self.metadata = md;
        self.committed = true;
        self.seal_for_serving()?;
        Ok(())
    }

    /// Reopen the lbug DB in READ_ONLY and downgrade the flock to shared so
    /// other process-local MCP instances can attach. Idempotent — safe to call
    /// when already sealed. Per ladybugdb concurrency docs, a READ_WRITE
    /// Database blocks all other process opens (even READ_ONLY), so the
    /// writer must explicitly release the write handle once initial build
    /// is committed.
    ///
    /// v1.5 PR2 audit bug #2 (foundation S-001 AS-002): the RW handle is
    /// `take()`-ed and dropped BEFORE the RO Database is constructed. lbug
    /// 0.16.1 docs forbid same-process RW+RO coexistence even briefly.
    /// v1.5 PR2 audit bug #3 (foundation S-001 AS-003): lock-downgrade
    /// errors propagate via `?` instead of `let _ = ...`.
    pub fn seal_for_serving(&mut self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        let db_path = self.layout.graph_db();

        // Step 1: drop the RW handle FIRST. take() leaves self.db = None.
        // The Drop impl of the old lbug::Database releases the file handle.
        let _rw_dropped = self.db.take();
        drop(_rw_dropped);
        debug_assert!(
            self.db.is_none(),
            "AS-002: RW Database must be dropped before opening RO"
        );

        // Step 2: now open the RO Database — no concurrent RW handle exists
        // in this process at this moment.
        let ro = lbug::Database::new(&db_path, lbug::SystemConfig::default().read_only(true))
            .map_err(|e| Error::Database(format!("seal: reopen graph.db read-only failed: {e}")))?;
        self.db = Some(ro);
        self.read_only = true;

        // Step 3: downgrade kernel flock to shared so peer MCP processes can
        // attach as readers. Propagate the error per AS-003.
        self._lock
            .downgrade_to_shared()
            .map_err(|e| Error::Database(format!("seal: flock downgrade failed: {e}")))?;
        Ok(())
    }

    /// Non-consuming commit for MCP session lifecycle. The MCP server needs
    /// `Store` to remain alive for `run_stdio` after the initial build, so
    /// it cannot use [`commit`] (which consumes). Without this path,
    /// metadata stays at `index_state=Building` forever → next boot's
    /// `cold_load` reports `CrashedBuilding` → nuke graph.db + rebuild loop.
    pub fn commit_in_place(&mut self) -> Result<()> {
        if self.read_only {
            return Err(Error::Other(anyhow::anyhow!(
                "Store is read-only (attached as reader to another writer); commit refused"
            )));
        }
        // v1.5 PR2 audit bug #1 (foundation S-001 AS-001): compute + persist
        // the Merkle root hash so the staleness gate (PR5) has its anchor.
        populate_root_hash(&mut self.metadata)?;
        // v1.5 PR4 staleness Phase B: bump graph_generation atomically
        // in lbug GraphMeta table (authoritative), then mirror to
        // metadata.json. Order: lbug write → metadata.json write → seal.
        bump_graph_generation(&mut self.metadata, self.db.as_ref())?;
        self.metadata.commit_in_place(&self.layout)?;
        self.committed = true;
        // v1.5 PR2 audit bug #3 (foundation S-001 AS-003): propagate seal
        // errors. If seal fails, the caller must know — otherwise the
        // exclusive flock is held forever and peer readers can never attach.
        self.seal_for_serving()?;
        Ok(())
    }

    /// v1.5 PR6.1a — consume the Store, nuke its cache, and return a
    /// freshly-opened Store ready for `build_index` + `commit_in_place`.
    ///
    /// **By-value `self`** is load-bearing: it forces the caller to
    /// surrender ownership, which guarantees the old `lbug::Database` +
    /// `LockFile` are dropped at function entry BEFORE we open the new
    /// Store. This preserves the AS-002 same-process drop-before-open
    /// ordering established by `seal_for_serving`.
    ///
    /// Lifecycle (each step crash-safe via the existing IndexState::Building
    /// sentinel — no new tombstone protocol needed at this layer):
    /// 1. Take self by value → old `db: Option<lbug::Database>` and
    ///    `_lock: LockFile` drop (releases handle + flock at function entry).
    /// 2. `nuke_cache_files(&layout)` — removes graph.db + .wal + sidecar.
    /// 3. `Store::open_with_root_and_schema` against the now-empty cache
    ///    → lands in `OpenOutcome::FreshBuild` (or `NoCache` / Mismatch /
    ///    CrashRecovery on edge cases).
    /// 4. Return the new Store. Caller is responsible for `build_index`
    ///    + `commit_in_place` to finalize. If the caller drops the new
    ///    Store without committing, metadata stays in `Building` state —
    ///    next process open lands on `RebuildCrashRecovery` (AS-005).
    ///
    /// **Cross-process flock + tombstone protocol** are PR6.1b — this method
    /// handles only same-process ownership transfer. PR6.1b layers
    /// `LockFile::try_acquire_exclusive` + `REBUILDING.tombstone` on top of
    /// this primitive.
    pub fn reindex_in_place(self, repo_root: &Path) -> Result<Self> {
        // Capture layout BEFORE dropping self — needed for nuke_cache_files.
        let layout = self.layout.clone();
        let schema_version = self.metadata.schema_version;

        // Step 1: drop the old Store. Explicit drop makes the ordering
        // obvious to readers; `self` would drop at function end anyway.
        drop(self);

        // Step 2: wipe cache files. layout.dir() and lock.pid remain on
        // disk (lock.pid will be re-acquired by step 3); graph.db and
        // metadata.json are removed.
        nuke_cache_files(&layout);

        // Step 3: reopen the cache. cache_root is the parent of the
        // per-repo dir (`layout.dir()`).
        let cache_root = layout
            .dir()
            .parent()
            .ok_or_else(|| {
                Error::Other(anyhow::anyhow!(
                    "reindex_in_place: layout dir has no parent (got {})",
                    layout.dir().display()
                ))
            })?
            .to_path_buf();
        Store::open_with_root_and_schema(&cache_root, repo_root, schema_version)
    }

    /// v1.5 PR4 Staleness Phase B (sub-spec staleness S-002) — reopen the
    /// lbug Database if a sibling writer bumped `graph_generation`
    /// on disk since this Store was opened.
    ///
    /// Returns:
    /// - `Ok(false)` — on-disk generation matches `self.metadata.graph_generation`;
    ///   no reopen performed.
    /// - `Ok(true)` — on-disk generation > cached; the lbug RO handle was
    ///   re-opened and `self.metadata.graph_generation` updated.
    /// - `Err(_)` — re-read of metadata.json failed (corrupt, perms, etc.)
    ///   OR lbug reopen failed. The Store is left in a usable state if
    ///   the failure was the metadata read; if the lbug reopen failed,
    ///   subsequent calls to `connection()` return Err until the next
    ///   successful reopen.
    ///
    /// Cheap fast-path: only reads metadata.json (small file, ~200B). The
    /// expensive lbug reopen runs only when the on-disk generation is
    /// strictly greater than the cached one.
    pub fn reopen_if_stale(&mut self) -> Result<bool> {
        // Cheap re-read of metadata.json. Failure here propagates without
        // mutating Store state.
        let on_disk = Metadata::load(&self.layout)?;
        if on_disk.graph_generation <= self.metadata.graph_generation {
            return Ok(false);
        }

        // Generation bumped → reopen the lbug Database.
        let db_path = self.layout.graph_db();

        // Drop the current handle FIRST (per AS-002 same-process constraint).
        let _drop_old = self.db.take();
        drop(_drop_old);

        // Reopen in the same mode the Store was using. `self.read_only`
        // already reflects that — a writer-mode Store shouldn't normally
        // need `reopen_if_stale` (it owns the generation), but we keep the
        // mode honest for completeness.
        let new_db = if self.read_only {
            lbug::Database::new(&db_path, lbug::SystemConfig::default().read_only(true))
        } else {
            lbug::Database::new(&db_path, lbug::SystemConfig::default())
        }
        .map_err(|e| Error::Database(format!("reopen_if_stale: lbug open failed: {e}")))?;

        self.db = Some(new_db);
        self.metadata = on_disk;
        Ok(true)
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // If we never committed, leave metadata in `building` state. On next
        // open, cold_load will detect it and trigger crash-recovery rebuild
        // (AS-025). No extra cleanup needed — the LockFile Drop removes lock.pid.
        let _ = self.committed;
    }
}

fn default_cache_root() -> Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("GRAPHATLAS_CACHE_DIR") {
        return Ok(PathBuf::from(override_dir));
    }
    let home = std::env::var("HOME").map_err(|_| Error::ConfigCorrupt {
        path: "$HOME".into(),
        reason: "HOME env var not set; cannot resolve default ~/.graphatlas".into(),
    })?;
    Ok(PathBuf::from(home).join(".graphatlas"))
}
