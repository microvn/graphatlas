//! v1.3-Tools-C10 — positional CSV column ordering pin.
//!
//! The indexer's COPY pipeline writes positional CSVs (no header). The struct
//! field order MUST match the DDL column order, else field values land in the
//! wrong columns. Tools-C10 mandates a `const COLUMNS: &[&str]` per
//! COPY-emitting struct that mirrors DDL order. This test asserts both that
//! the const exists AND that it lists every v4 column in the expected order.

use ga_query::indexer::{FILE_COLUMNS, SYMBOL_COLUMNS};

#[test]
fn symbol_columns_const_exists_and_matches_v4_ddl_order() {
    // v4 Symbol DDL column order — keep in sync with crates/ga-index/src/schema.rs
    // BASE_DDL_STATEMENTS Symbol CREATE NODE TABLE clause.
    // v1.3 ships 11 v4 scalar/boolean/string cols. Composites
    // (params STRUCT[], modifiers LIST<STRING>) deferred to PR5 per
    // Tools-C13 — kuzu#6045 family hits empty-cache reopen lifecycle when
    // composite DEFAULTs are present. PR5 lands ALTER + populated CSV
    // emission together (full-row COPY, no column-list omit).
    let expected = [
        "id",
        "name",
        "file",
        "kind",
        "line",
        "line_end",
        "qualified_name",
        "return_type",
        "arity",
        "is_async",
        "is_override",
        "is_abstract",
        "is_static",
        "is_test_marker",
        "is_generated",
        "confidence",
        "doc_summary",
        // PR5c1 — composite cols. doc_summary still pending PR-future.
        "modifiers",
        "params",
    ];
    assert_eq!(
        SYMBOL_COLUMNS, expected,
        "SYMBOL_COLUMNS must match v4 DDL column order (Tools-C10)"
    );
}

#[test]
fn file_columns_const_exists_and_matches_v4_ddl_order() {
    // v4 File DDL column order
    let expected = [
        // v3 columns
        "path",
        "lang",
        "size",
        // v4 additions per spec §"File node — 5 new columns"
        "sha256",
        "modified_at",
        "loc",
        "is_generated",
        "is_vendored",
    ];
    assert_eq!(
        FILE_COLUMNS, expected,
        "FILE_COLUMNS must match v4 DDL column order (Tools-C10)"
    );
}
