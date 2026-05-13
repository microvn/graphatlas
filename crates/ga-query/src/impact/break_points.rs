//! Cluster C3 — break-point discovery from CALLS edges.

use super::types::BreakPoint;
use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use std::collections::HashMap;

/// Every CALLS edge whose callee is `symbol`, surfaced as a `BreakPoint`.
///
/// One entry per `(file, call_site_line)`; `caller_symbols` collects every
/// caller name that shares that exact line (usually one, but polymorphic
/// overload resolution can lead to multiple). Deterministic ordering:
/// ascending by `(file, line)`.
///
/// Only CALLS is considered for break points — REFERENCES sites are not
/// call sites and don't break when the symbol changes its signature.
pub(super) fn collect_break_points(store: &Store, symbol: &str) -> Result<Vec<BreakPoint>> {
    if !common::is_safe_ident(symbol) {
        return Ok(Vec::new());
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let cypher = format!(
        "MATCH (caller:Symbol)-[r:CALLS]->(callee:Symbol) \
         WHERE callee.name = '{symbol}' AND caller.kind <> 'external' \
         RETURN caller.file, r.call_site_line, caller.name"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("break-points query: {e}")))?;

    // Group by (file, line) so same-line polymorphic callers merge.
    let mut grouped: HashMap<(String, u32), Vec<String>> = HashMap::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 3 {
            continue;
        }
        let file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let caller = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        grouped.entry((file, line)).or_default().push(caller);
    }

    let mut out: Vec<BreakPoint> = grouped
        .into_iter()
        .map(|((file, line), mut callers)| {
            callers.sort();
            callers.dedup();
            BreakPoint {
                file,
                line,
                caller_symbols: callers,
            }
        })
        .collect();
    out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    Ok(out)
}
