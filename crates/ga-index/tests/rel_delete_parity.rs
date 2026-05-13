//! v1.4 Tools-C21 — REL CREATE/DELETE parity invariant.
//!
//! Architectural test: every CREATE REL TABLE in BASE_DDL_STATEMENTS
//! MUST have a matching DELETE statement in REL_DELETE_STATEMENTS so
//! reindex is idempotent (no stale REL rows pointing at deleted Symbol
//! ids after a re-emit). Without this invariant, every new REL added to
//! schema.rs needs the dev to remember the matching DELETE — easy miss
//! that produces silent stale-edge bugs.
//!
//! Spec: graphatlas-v1.4-data-model.md AS-005 + Tools-C21.

use ga_index::schema::{BASE_DDL_STATEMENTS, REL_DELETE_STATEMENTS};
use std::collections::HashSet;

/// Pull the REL name from `CREATE REL TABLE IF NOT EXISTS X (...)`.
fn rel_name_from_create(stmt: &str) -> Option<&str> {
    let prefix = "CREATE REL TABLE IF NOT EXISTS ";
    let rest = stmt.trim().strip_prefix(prefix)?;
    // REL name ends at first whitespace or `(`.
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Pull the REL name from `MATCH ()-[r:X]->() DELETE r`.
fn rel_name_from_delete(stmt: &str) -> Option<&str> {
    let after_bracket = stmt.find("[r:")? + 3;
    let rest = &stmt[after_bracket..];
    let end = rest.find(']')?;
    Some(&rest[..end])
}

#[test]
fn every_create_rel_table_has_matching_delete_statement() {
    let creates: HashSet<&str> = BASE_DDL_STATEMENTS
        .iter()
        .filter_map(|stmt| rel_name_from_create(stmt))
        .collect();

    let deletes: HashSet<&str> = REL_DELETE_STATEMENTS
        .iter()
        .filter_map(|stmt| rel_name_from_delete(stmt))
        .collect();

    assert_eq!(
        creates,
        deletes,
        "Tools-C21 violation: every CREATE REL TABLE must have a matching \
         DELETE statement. CREATE-only (missing DELETE): {:?}; DELETE-only \
         (orphaned): {:?}",
        creates.difference(&deletes).collect::<Vec<_>>(),
        deletes.difference(&creates).collect::<Vec<_>>(),
    );
}

#[test]
fn rel_delete_statements_is_non_empty() {
    // Sanity guard — if someone refactors the const to a different shape
    // (e.g. function), this catches it before the parity test runs and
    // misleads with an empty-set false-positive.
    assert!(
        !REL_DELETE_STATEMENTS.is_empty(),
        "REL_DELETE_STATEMENTS must contain at least the v3 catch-alls"
    );
}

#[test]
fn create_parser_recognises_v4_rel_tables() {
    // Pin the parser shape — if BASE_DDL_STATEMENTS changes format
    // (e.g. trailing whitespace after table name), the test that uses
    // rel_name_from_create silently degrades. Spot-check a handful of
    // known v4 RELs.
    let creates: HashSet<&str> = BASE_DDL_STATEMENTS
        .iter()
        .filter_map(|stmt| rel_name_from_create(stmt))
        .collect();
    for expected in [
        "IMPORTS_NAMED",
        "IMPLEMENTS",
        "DECORATES",
        "CALLS_HEURISTIC",
    ] {
        assert!(
            creates.contains(expected),
            "BASE_DDL_STATEMENTS missing {expected} (v4-shipped REL)"
        );
    }
}

#[test]
fn delete_parser_recognises_v4_rel_tables() {
    let deletes: HashSet<&str> = REL_DELETE_STATEMENTS
        .iter()
        .filter_map(|stmt| rel_name_from_delete(stmt))
        .collect();
    for expected in [
        "IMPORTS_NAMED",
        "IMPLEMENTS",
        "DECORATES",
        "CALLS_HEURISTIC",
    ] {
        assert!(
            deletes.contains(expected),
            "REL_DELETE_STATEMENTS missing {expected} (v4-shipped REL)"
        );
    }
}
