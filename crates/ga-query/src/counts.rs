//! ga-ui Spec A S-003 follow-up — post-reindex tally helpers.
//!
//! `compute_index_counts(store, duration_ms)` and
//! `compute_health_summary(store)` are called by the reindex pipeline
//! after `Store::commit` (Building → Complete). The values land in
//! `Metadata::set_index_counts` / `set_health_summary` so the
//! projects-list endpoint can render them without a per-row lbug
//! round-trip.
//!
//! Best-effort: any individual COUNT failure is logged and the field
//! left at 0 (the dashboard prefers "0" over a missing tile).

use ga_core::{Error, HealthSummary, IndexCounts, Result};
use ga_index::Store;

/// Walk the cache dir for a real `db_size_bytes` measure. Mirrors the
/// shape `ga_index::list::list_caches` reports.
fn dir_size_bytes(dir: &std::path::Path) -> u64 {
    fn walk(p: &std::path::Path, acc: &mut u64) {
        let entries = match std::fs::read_dir(p) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_file() => {
                    if let Ok(m) = entry.metadata() {
                        *acc += m.len();
                    }
                }
                Ok(ft) if ft.is_dir() => walk(&path, acc),
                _ => {}
            }
        }
    }
    let mut total = 0u64;
    walk(dir, &mut total);
    total
}

fn count_cypher(store: &Store, cypher: &str) -> Result<u64> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("count query {cypher}: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n.max(0) as u64);
        }
    }
    Ok(0)
}

/// Best-effort count — returns 0 + logs on failure rather than
/// propagating the error. Reindex isn't supposed to fail just because
/// one tally couldn't compute.
fn count_or_zero(store: &Store, label: &str, cypher: &str) -> u64 {
    match count_cypher(store, cypher) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(target: "ga_query::counts", "{label}: {e}");
            0
        }
    }
}

/// Spec A AS-026 — index size tallies. `duration_ms` is the wall-clock
/// the reindex pipeline already tracks; `db_size_bytes` is sampled via
/// recursive filesystem walk of the cache dir.
pub fn compute_index_counts(store: &Store, duration_ms: u64) -> IndexCounts {
    let node_count = count_or_zero(store, "node_count", "MATCH (s:Symbol) RETURN count(s)");
    // Edge total — sum all REL tables we ship. Lbug Cypher can't
    // OR-match across REL tables in a single MATCH (different schema),
    // so we sum per-kind counts.
    let edge_kinds = [
        ("CALLS", "MATCH ()-[r:CALLS]->() RETURN count(r)"),
        ("CALLS_HEURISTIC", "MATCH ()-[r:CALLS_HEURISTIC]->() RETURN count(r)"),
        ("IMPORTS", "MATCH ()-[r:IMPORTS]->() RETURN count(r)"),
        ("IMPORTS_NAMED", "MATCH ()-[r:IMPORTS_NAMED]->() RETURN count(r)"),
        ("DEFINES", "MATCH ()-[r:DEFINES]->() RETURN count(r)"),
        ("CONTAINS", "MATCH ()-[r:CONTAINS]->() RETURN count(r)"),
        ("EXTENDS", "MATCH ()-[r:EXTENDS]->() RETURN count(r)"),
        ("IMPLEMENTS", "MATCH ()-[r:IMPLEMENTS]->() RETURN count(r)"),
        ("OVERRIDES", "MATCH ()-[r:OVERRIDES]->() RETURN count(r)"),
        ("REFERENCES", "MATCH ()-[r:REFERENCES]->() RETURN count(r)"),
        ("DECORATES", "MATCH ()-[r:DECORATES]->() RETURN count(r)"),
        ("TESTED_BY", "MATCH ()-[r:TESTED_BY]->() RETURN count(r)"),
        ("MODULE_TYPED", "MATCH ()-[r:MODULE_TYPED]->() RETURN count(r)"),
    ];
    let mut edge_count = 0u64;
    for (label, cypher) in edge_kinds {
        edge_count = edge_count.saturating_add(count_or_zero(store, label, cypher));
    }
    let file_count = count_or_zero(store, "file_count", "MATCH (f:File) RETURN count(f)");
    let db_size_bytes = dir_size_bytes(store.layout().dir());

    IndexCounts {
        node_count,
        edge_count,
        file_count,
        last_index_duration_ms: duration_ms,
        db_size_bytes,
    }
}

/// Spec A AS-027 — health tally. Definitions match the projects-list
/// dashboard's "rough sanity numbers" intent, not exact spec parity
/// with `ga_query::hubs` / `bridges` etc (those return ranked entries;
/// we just need scalar counts here).
///
/// Definitions:
/// - hubs_count           — symbols with CALLS in-degree ≥ 10 (rough; matches
///                          the dashboard's "high-traffic node" intuition).
/// - bridges_count        — left at 0; an exact cut-vertex computation
///                          is heavy and the dashboard already shows hubs.
///                          Phase 2 can swap in `ga_query::bridges` if
///                          users miss the signal.
/// - dead_code_count      — symbols never CALLED and never DEFINES-target
///                          via tests (loose proxy for `ga_dead_code`).
/// - large_functions_count — symbols with `line_end - line + 1 ≥ 50`.
/// - tested_count         — symbols with ≥ 1 incoming TESTED_BY edge.
pub fn compute_health_summary(store: &Store) -> HealthSummary {
    let hubs_count = count_or_zero(
        store,
        "hubs_count",
        "MATCH (s:Symbol)<-[c:CALLS|:CALLS_HEURISTIC]-() WITH s, count(c) AS deg WHERE deg >= 10 RETURN count(s)",
    );
    // Bridges deferred to a follow-up (proper cut-vertex algo).
    let bridges_count = 0u64;
    let dead_code_count = count_or_zero(
        store,
        "dead_code_count",
        "MATCH (s:Symbol) WHERE NOT EXISTS { MATCH ()-[:CALLS|:CALLS_HEURISTIC|:REFERENCES]->(s) } RETURN count(s)",
    );
    let large_functions_count = count_or_zero(
        store,
        "large_functions_count",
        "MATCH (s:Symbol) WHERE s.line_end - s.line + 1 >= 50 RETURN count(s)",
    );
    let tested_count = count_or_zero(
        store,
        "tested_count",
        "MATCH (s:Symbol)<-[:TESTED_BY]-() RETURN count(s)",
    );

    HealthSummary {
        computed_at_unix: now_unix(),
        hubs_count,
        bridges_count,
        dead_code_count,
        large_functions_count,
        tested_count,
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure-logic test: `dir_size_bytes` against a tempdir.
    #[test]
    fn dir_size_sums_recursive_file_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.bin"), vec![0u8; 100]).unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.bin"), vec![0u8; 250]).unwrap();
        std::fs::write(sub.join("c.bin"), vec![0u8; 1]).unwrap();
        assert_eq!(dir_size_bytes(tmp.path()), 351);
    }

    #[test]
    fn dir_size_returns_zero_for_missing_path() {
        assert_eq!(dir_size_bytes(std::path::Path::new("/tmp/__no_exist_xyz__")), 0);
    }
}
