//! AS-008/AS-025/AS-027 spec-literal rebuild log lines.
//!
//! Extracted from `store.rs` to keep store.rs focused on lifecycle. These
//! strings are byte-matched by bench tests + operator scrapers — format
//! changes are spec changes (Major) per the constraint in
//! `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-foundation.md`.

/// AS-008 spec-literal rebuild log line.
pub fn rebuild_log_line_schema_mismatch(cache: u32, binary: u32) -> String {
    format!("schema version mismatch (cache={cache}, binary={binary}), rebuilding")
}

/// AS-027 spec-literal rebuild progress line.
pub fn rebuild_log_line_schema_upgrade(binary: u32) -> String {
    format!("Rebuilding cache for schema v{binary} (estimated ~3 min)...")
}

/// AS-025 crash-recovery user-visible line.
pub fn rebuild_log_line_crash_recovery() -> String {
    "previous index was incomplete (crash recovery), rebuilding from source".to_string()
}
