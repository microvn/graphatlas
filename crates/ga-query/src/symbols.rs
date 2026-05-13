//! Tools S-004 — ga_symbols. Pattern-matched symbol search with caller-count
//! relevance boost (AS-008) and Levenshtein fuzzy mode (AS-009). Fresh / empty
//! index returns [`Error::IndexNotReady`] for AS-010.

use crate::common::{is_safe_ident, levenshtein};
use crate::{SymbolEntry, SymbolsMeta, SymbolsResponse};
use ga_core::{Error, Result};
use ga_index::Store;

/// Matching mode for [`symbols`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolsMatch {
    /// Case-sensitive exact match on the symbol name.
    Exact,
    /// Levenshtein-ranked fuzzy match across all symbol names.
    Fuzzy,
}

/// AS-008/AS-009/AS-010. Cap = 10 per Tools-C5 guidance (atomic tool, LLM-sized
/// output). `meta.truncated` / `meta.total_available` surface the pre-cap total.
pub fn symbols(store: &Store, pattern: &str, mode: SymbolsMatch) -> Result<SymbolsResponse> {
    // Tools-C9-d allowlist on pattern — exact mode needs ident-safe input
    // because the value goes into a Cypher equality; fuzzy mode does too so we
    // keep the boundary uniform (the adversarial corpus tests both).
    if !is_safe_ident(pattern) {
        return Ok(SymbolsResponse::default());
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // AS-010 — empty graph means indexing hasn't populated data yet. Return
    // typed IndexNotReady so the MCP server maps to -32000 for the client.
    if graph_is_empty(&conn)? {
        return Err(Error::IndexNotReady {
            status: "indexing".to_string(),
            progress: 0.0,
        });
    }

    let candidates = match mode {
        SymbolsMatch::Exact => collect_exact(&conn, pattern)?,
        SymbolsMatch::Fuzzy => collect_fuzzy(&conn, pattern)?,
    };
    let total_available = candidates.len() as u32;

    const CAP: usize = 10;
    let truncated = candidates.len() > CAP;
    let symbols_out: Vec<SymbolEntry> = candidates.into_iter().take(CAP).collect();

    Ok(SymbolsResponse {
        symbols: symbols_out,
        meta: SymbolsMeta {
            truncated,
            total_available,
        },
    })
}

fn graph_is_empty(conn: &lbug::Connection<'_>) -> Result<bool> {
    let rs = conn
        .query("MATCH (s:Symbol) RETURN count(s)")
        .map_err(|e| Error::Other(anyhow::anyhow!("symbol count: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n == 0);
        }
    }
    Ok(true)
}

fn collect_exact(conn: &lbug::Connection<'_>, pattern: &str) -> Result<Vec<SymbolEntry>> {
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{}' AND s.kind <> 'external' \
         RETURN s.name, s.kind, s.file, s.line",
        pattern,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("symbols exact query: {e}")))?;

    let mut rows: Vec<SymbolEntry> = Vec::new();
    for row in rs {
        if let Some(entry) = row_to_entry(row, 1.0) {
            rows.push(entry);
        }
    }
    rank_by_callers(conn, &mut rows)?;
    Ok(rows)
}

fn collect_fuzzy(conn: &lbug::Connection<'_>, pattern: &str) -> Result<Vec<SymbolEntry>> {
    // Full scan is fine at M1 scale (≤50k symbols per AS-008) — same bound
    // suggest_similar uses.
    let cypher =
        "MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.kind, s.file, s.line";
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("symbols fuzzy query: {e}")))?;

    let mut scored: Vec<(u32, SymbolEntry)> = Vec::new();
    for row in rs {
        if let Some(entry) = row_to_entry(row, 0.0) {
            let d = levenshtein(pattern, &entry.name);
            scored.push((d, entry));
        }
    }
    // Stable sort by Levenshtein distance (asc), then name — deterministic ties.
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let max_d = scored.last().map(|(d, _)| *d).unwrap_or(1).max(1) as f32;
    Ok(scored
        .into_iter()
        .map(|(d, mut e)| {
            // Normalize score so closer = higher. 1.0 for exact, drops with d.
            e.score = 1.0 - (d as f32 / max_d) * 0.5;
            e
        })
        .collect())
}

fn row_to_entry<R: IntoIterator<Item = lbug::Value>>(
    row: R,
    default_score: f32,
) -> Option<SymbolEntry> {
    let cols: Vec<lbug::Value> = row.into_iter().collect();
    if cols.len() < 4 {
        return None;
    }
    let name = match &cols[0] {
        lbug::Value::String(s) => s.clone(),
        _ => return None,
    };
    let kind = match &cols[1] {
        lbug::Value::String(s) => s.clone(),
        _ => String::from("other"),
    };
    let file = match &cols[2] {
        lbug::Value::String(s) => s.clone(),
        _ => return None,
    };
    let line = match &cols[3] {
        lbug::Value::Int64(n) => *n as u32,
        _ => 0,
    };
    Some(SymbolEntry {
        name,
        kind,
        file,
        line,
        score: default_score,
    })
}

fn rank_by_callers(conn: &lbug::Connection<'_>, rows: &mut [SymbolEntry]) -> Result<()> {
    // Exact-match set is ≤ a handful of files; one query per is acceptable.
    // Score = 1.0 base + 0.05 * caller_count (capped at 2.0). The exact form
    // doesn't matter — tests only assert monotonicity — but the bump keeps
    // scores comparable across signals.
    for e in rows.iter_mut() {
        let cypher = format!(
            "MATCH (caller:Symbol)-[:CALLS]->(callee:Symbol) \
             WHERE callee.name = '{}' AND callee.file = '{}' RETURN count(caller)",
            e.name, e.file,
        );
        let rs = conn
            .query(&cypher)
            .map_err(|err| Error::Other(anyhow::anyhow!("caller-count: {err}")))?;
        let mut n = 0i64;
        for row in rs {
            if let Some(lbug::Value::Int64(v)) = row.into_iter().next() {
                n = v;
            }
        }
        e.score = (1.0 + 0.05 * n as f32).min(2.0);
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
    });
    Ok(())
}
