//! Graph schema — lbug DDL executed on every `Store::open`.
//!
//! v1.3 (schema v4) — split per Tools-C9, single-phase in practice:
//!
//! - [`BASE_DDL_STATEMENTS`]: idempotent CREATE … IF NOT EXISTS forms.
//!   Replayed every `Store::open`. **Holds the full v4 schema** including
//!   v4 NODE columns with DEFAULTs (Tools-C12 DOUBLE workaround for FLOAT;
//!   composites — `params`, `modifiers` — deferred to PR5 per Tools-C13).
//! - [`MIGRATION_STATEMENTS`]: ALTER ADD COLUMN forms. **EMPTY in v1.3 PR1
//!   and PR2.** Reserved for future schema bumps where ALTER is needed on
//!   already-deployed v4 caches without nuke + rebuild.
//!
//! ## Why ALTER pattern was tested but reverted (PR2 finding)
//!
//! PR2 attempted to move v4 NODE columns from `BASE_DDL_STATEMENTS`
//! CREATE-with-DEFAULT into `MIGRATION_STATEMENTS` ALTER ADD with DEFAULT,
//! based on Path H/J/K of `examples/repro_alter_migration_real.rs` proving
//! ALTER pattern works through the real reindex lifecycle including
//! composites (un-deferring `params`/`modifiers`).
//!
//! **Empirically the ALTER pattern triggers kuzu#6045 family bug** across
//! the existing graphatlas test harness — `OverflowFile::checkpoint()`
//! corrupts `PrimaryKeyIndexStorageInfo` when an empty/sparse cache is
//! committed and reopened. The bug fires because ALTER ADD COLUMN performs
//! additional checkpoint operations that overwhelm the OverflowFile
//! initialization path. Production `build_index` lifecycle (open → walk +
//! parse + COPY → commit) populates the cache before commit and doesn't
//! trip the bug, but the test harness commits empty/sparse caches in many
//! reopen-pattern tests that all break.
//!
//! Net user-value: ALTER pattern over CREATE-with-DEFAULT in v1.3 is **0**
//! because:
//! - Both produce the same final v4 schema.
//! - MIGRATION_STATEMENTS only runs on `!Resumed` (FreshBuild,
//!   RebuildSchemaMismatch, RebuildCrashRecovery) — all 3 cases have
//!   empty caches that the indexer immediately populates. Identical lifecycle
//!   to CREATE-with-DEFAULT.
//! - Composites un-defer is a v1.3-PR5-internal optimization (PR5 can ALTER
//!   ADD them itself when it ships the populated CSV emission).
//!
//! So PR2 keeps PR1's CREATE-with-DEFAULT pattern. v1.3-Tools-C13 (defer
//! composites) stays. ALTER incremental migration deferred to v5+ when
//! kuzu#6045 / kuzu#5159 family fixes land upstream.

/// Always-replayed schema statements. Idempotent via `IF NOT EXISTS`.
/// Order matters: REL tables reference NODE tables declared earlier.
pub const BASE_DDL_STATEMENTS: &[&str] = &[
    // ---- v1.5 PR4 staleness Phase B — generation counter table ----
    //
    // `GraphMeta` holds bookkeeping rows keyed by string. v1.5 uses one
    // row: `key="graph_generation", value="<u64 as string>"`. Stored in
    // lbug so the bump is atomic-with-data per multi-voice challenge C-2
    // (lbug transaction commits data + generation together; metadata.json
    // is a mirror cache, not the source of truth).
    //
    // Why `GraphMeta` not `_Meta`: lbug Cypher parser may treat
    // leading-underscore identifiers specially in some versions; safer to
    // ship an unambiguous PascalCase name.
    "CREATE NODE TABLE IF NOT EXISTS GraphMeta (\
        key STRING, \
        value STRING, \
        PRIMARY KEY (key))",
    // ---- NODE tables (v4 final shape) ----
    "CREATE NODE TABLE IF NOT EXISTS File (\
        path STRING, \
        lang STRING, \
        size INT64, \
        sha256 BLOB, \
        modified_at TIMESTAMP, \
        loc INT64 DEFAULT 0, \
        is_generated BOOLEAN DEFAULT false, \
        is_vendored BOOLEAN DEFAULT false, \
        PRIMARY KEY (path))",
    // Symbol v4: 13 scalar/boolean/string + 2 composite cols (PR5c).
    // - confidence: DOUBLE not FLOAT per Tools-C12 (kuzu#5159 bites FLOAT).
    // - params STRUCT(...)[] + modifiers STRING[]: shipped PR5c via
    //   CREATE-with-DEFAULT path (Tools-C13 superseded). Empirical
    //   verification: spike_pr5c_store.rs T1-T5 (5/5 PASS) — PR2's
    //   kuzu#6045 trap was ALTER-ADD-specific; CREATE-with-DEFAULT for
    //   composites survives empty-cache reopen + DDL-replay lifecycle.
    "CREATE NODE TABLE IF NOT EXISTS Symbol (\
        id STRING, \
        name STRING, \
        file STRING, \
        kind STRING, \
        line INT64, \
        line_end INT64, \
        qualified_name STRING DEFAULT '', \
        return_type STRING DEFAULT '', \
        arity INT64 DEFAULT -1, \
        is_async BOOLEAN DEFAULT false, \
        is_override BOOLEAN DEFAULT false, \
        is_abstract BOOLEAN DEFAULT false, \
        is_static BOOLEAN DEFAULT false, \
        is_test_marker BOOLEAN DEFAULT false, \
        is_generated BOOLEAN DEFAULT false, \
        confidence DOUBLE DEFAULT 1.0, \
        doc_summary STRING DEFAULT '', \
        has_unresolved_override BOOLEAN DEFAULT false, \
        modifiers STRING[] DEFAULT CAST([] AS STRING[]), \
        params STRUCT(name STRING, type STRING, default_value STRING)[] \
            DEFAULT CAST([] AS STRUCT(name STRING, type STRING, default_value STRING)[]), \
        PRIMARY KEY (id))",
    // ---- v3 REL tables (catch-alls preserved per Tools-C7) ----
    "CREATE REL TABLE IF NOT EXISTS IMPORTS  (FROM File   TO File, import_line INT64, imported_names STRING, re_export BOOLEAN)",
    "CREATE REL TABLE IF NOT EXISTS DEFINES  (FROM File   TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS CONTAINS (FROM Symbol TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS CALLS    (FROM Symbol TO Symbol, call_site_line INT64)",
    "CREATE REL TABLE IF NOT EXISTS EXTENDS  (FROM Symbol TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS TESTED_BY(FROM Symbol TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS REFERENCES(FROM Symbol TO Symbol, ref_site_line INT64, ref_kind STRING)",
    "CREATE REL TABLE IF NOT EXISTS MODULE_TYPED(FROM File TO Symbol)",
    // ---- v4 typed REL variants (additive) ----
    "CREATE REL TABLE IF NOT EXISTS CALLS_HEURISTIC(FROM Symbol TO Symbol, call_site_line INT64)",
    "CREATE REL TABLE IF NOT EXISTS IMPLEMENTS(FROM Symbol TO Symbol)",
    "CREATE REL TABLE IF NOT EXISTS IMPORTS_NAMED(\
        FROM File TO Symbol, \
        import_line INT64, \
        alias STRING DEFAULT '', \
        re_export BOOLEAN DEFAULT false, \
        re_export_source STRING DEFAULT '', \
        is_type_only BOOLEAN DEFAULT false)",
    "CREATE REL TABLE IF NOT EXISTS DECORATES(FROM Symbol TO Symbol, decorator_args STRING DEFAULT '')",
    // ---- v1.4 (S-001a) ----
    // OVERRIDES: subclass-method overrides parent-method. Tools-C18 class-level
    // witness: every OVERRIDES(child_method, parent_method) implies a
    // corresponding (child_class, parent_class) edge in EXTENDS or IMPLEMENTS
    // catch-alls (Java EXTENDS, Rust IMPLEMENTS for trait-impl). NOT a
    // row-for-row strict union (endpoints differ). Resolution rules + AT-011
    // audit query in graphatlas-v1.4-data-model.md §"Constraints &
    // Invariants".
    "CREATE REL TABLE IF NOT EXISTS OVERRIDES(FROM Symbol TO Symbol)",
];

/// Migration-only statements. EMPTY in v1.3 PR1 and PR2 — see module doc.
/// Reserved for future schema bumps where true incremental ALTER is needed
/// on already-deployed caches (post-kuzu#6045 fix upstream).
pub const MIGRATION_STATEMENTS: &[&str] = &[];

/// Backward-compat alias. Equals BASE_DDL_STATEMENTS in v1.3.
pub const DDL_STATEMENTS: &[&str] = BASE_DDL_STATEMENTS;

/// v1.4 Tools-C21 — REL CREATE/DELETE parity invariant. Every CREATE REL
/// TABLE in `BASE_DDL_STATEMENTS` MUST have a matching `MATCH ()-[r:X]->()
/// DELETE r` statement here so reindex is idempotent (no stale REL rows
/// pointing at deleted Symbol ids after a re-emit). Without this list,
/// every new REL added to schema needs the dev to remember the matching
/// DELETE in indexer.rs — easy miss that produces silent stale-edge bugs.
///
/// Architectural test: `crates/ga-index/tests/rel_delete_parity.rs` parses
/// REL names from both `BASE_DDL_STATEMENTS` and this list and asserts the
/// sets match. `crates/ga-query/src/indexer.rs` iterates this list during
/// the pre-COPY purge phase.
pub const REL_DELETE_STATEMENTS: &[&str] = &[
    // v3 catch-alls (Tools-C7 strict-union family)
    "MATCH ()-[r:IMPORTS]->() DELETE r",
    "MATCH ()-[r:DEFINES]->() DELETE r",
    "MATCH ()-[r:CONTAINS]->() DELETE r",
    "MATCH ()-[r:CALLS]->() DELETE r",
    "MATCH ()-[r:EXTENDS]->() DELETE r",
    "MATCH ()-[r:TESTED_BY]->() DELETE r",
    "MATCH ()-[r:REFERENCES]->() DELETE r",
    "MATCH ()-[r:MODULE_TYPED]->() DELETE r",
    // v4 typed variants
    "MATCH ()-[r:CALLS_HEURISTIC]->() DELETE r",
    "MATCH ()-[r:IMPLEMENTS]->() DELETE r",
    "MATCH ()-[r:IMPORTS_NAMED]->() DELETE r",
    "MATCH ()-[r:DECORATES]->() DELETE r",
    // v1.4 (S-001a) Tools-C21 parity invariant — keep in lock-step with
    // BASE_DDL_STATEMENTS additions above.
    "MATCH ()-[r:OVERRIDES]->() DELETE r",
];
