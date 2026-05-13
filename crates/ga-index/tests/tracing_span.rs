//! v1.5 PR3 foundation S-004 AS-013 — `reindex_span` correlation_id field.
//!
//! AS-012 subscriber wiring is tested via the `graphatlas` binary entry
//! (src/main.rs `init_tracing`); this file validates the span helper that
//! PR6 (ga_reindex tool) will consume.

use ga_index::lifecycle_helpers::reindex_span;

#[test]
fn reindex_span_returns_unique_correlation_ids() {
    // AS-013: each call must mint a fresh UUID so independent reindex
    // invocations are distinguishable in structured logs.
    let (_, id1) = reindex_span();
    let (_, id2) = reindex_span();
    assert_ne!(
        id1, id2,
        "reindex_span must mint a new correlation_id per call; got duplicate {id1}"
    );
}

#[test]
fn reindex_span_correlation_id_is_uuid_v4() {
    // AS-013: correlation_id surfaces in MCP response so clients can
    // tie back to log spans. Stable UUIDv4 wire format.
    let (_, id) = reindex_span();
    assert_eq!(
        id.get_version_num(),
        4,
        "reindex_span correlation_id must be UUIDv4, got version {}",
        id.get_version_num()
    );
}

#[test]
fn reindex_span_can_be_entered_and_dropped() {
    // AS-013: returned span must be usable with `.enter()` pattern that
    // PR6 will adopt around its full-rebuild + commit sequence.
    let (span, _id) = reindex_span();
    {
        let _guard = span.enter();
        // Emit a log event inside the span — when a subscriber is installed
        // it would carry the correlation_id field. With no subscriber the
        // event is dropped at near-zero cost.
        tracing::info!("inside reindex span");
    }
    // Span guard dropped without panic.
}
