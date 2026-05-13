//! Persistent graph index on LadybugDB (`lbug 0.15`).
//! S-003 wires cache layout + metadata + PID lock + lbug Store.
//! Future sub-modules (S-005) will add Merkle staleness detection + ga_reindex.

pub mod cache;
mod generation;
pub mod lifecycle_helpers;
pub mod list;
pub mod lock;
pub mod log_lines;
pub mod metadata;
pub mod monorepo;
mod nuke;
mod root_hash;
pub mod schema;
pub mod store;

pub use log_lines::{
    rebuild_log_line_crash_recovery, rebuild_log_line_schema_mismatch,
    rebuild_log_line_schema_upgrade,
};
pub use store::{OpenOutcome, Store};

/// v1.3 (schema v4) — Symbol gains 13 new columns (qualified_name, params,
/// signature, denormalized booleans, confidence, doc_summary), File gains 5
/// (sha256, modified_at, loc, is_generated, is_vendored). Four typed REL
/// variants added (IMPLEMENTS, CALLS_HEURISTIC, IMPORTS_NAMED, DECORATES);
/// v3 catch-all CALLS / EXTENDS / IMPORTS / REFERENCES preserved per
/// v1.3-Tools-C7 strict-union invariant.
///
/// PR1 ships the schema scaffolding with `MIGRATION_STATEMENTS` empty —
/// caches from schema_version<4 are still deleted + rebuilt on next open via
/// the existing `cold_load` mismatch path. PR2 (S-001) wires
/// `run_schema_migration()` for ALTER-incremental upgrades.
///
/// v1.4 (Tools-C20): bumped 4→5 for the OVERRIDES REL +
/// `Symbol.has_unresolved_override` column ship of S-001a. Markdown label
/// "v5" alone has no runtime effect; the integer here is what cold_load
/// compares — bumping it is the ONLY supported v1.3→v1.4 migration path
/// per Tools-C9 `MIGRATION_STATEMENTS = []` (ALTER deferred to v6+ pending
/// kuzu#6045 fix). See `crates/ga-index/tests/schema_v5.rs` for the pin
/// + cold_load mismatch acceptance tests.
pub const SCHEMA_VERSION: u32 = 5;
