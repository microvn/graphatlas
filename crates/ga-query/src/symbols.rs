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
    /// Case-insensitive substring match with prefix-priority ranking.
    /// Intended for HTTP search-as-you-type — caps at 50 (Spec E C-2).
    Contains,
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
        SymbolsMatch::Contains => collect_contains(&conn, pattern)?,
    };
    let total_available = candidates.len() as u32;

    // CAP per mode: Exact/Fuzzy serve MCP (LLM-sized output, Tools-C5);
    // Contains serves the HTTP search dropdown (Spec E C-2 = 50 hits).
    let cap = match mode {
        SymbolsMatch::Exact | SymbolsMatch::Fuzzy => 10,
        SymbolsMatch::Contains => 50,
    };
    let truncated = candidates.len() > cap;
    let symbols_out: Vec<SymbolEntry> = candidates.into_iter().take(cap).collect();

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
    //
    // P2.2 (2026-05-22) — pure Levenshtein over-penalized longer substring
    // matches (e.g. "overflow" → "overflowBehavior" had d=8 because the 8
    // trailing chars cost; but "error" had d=5-7 to "overflow", so "error"
    // ranked HIGHER than the actual substring match). Fix: blend
    // Levenshtein with substring/prefix bonuses so name.contains(pattern)
    // always outranks non-containing equidistant noise. Case-insensitive
    // comparison so `Overflow` matches lowercase pattern `overflow`.
    let cypher =
        "MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.kind, s.file, s.line";
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("symbols fuzzy query: {e}")))?;

    let needle = pattern.to_lowercase();
    let mut scored: Vec<(f32, u32, SymbolEntry)> = Vec::new();
    for row in rs {
        if let Some(entry) = row_to_entry(row, 0.0) {
            let d = levenshtein(pattern, &entry.name);
            let name_lc = entry.name.to_lowercase();
            // Substring bonus: 0.5 for prefix match (very specific signal),
            // 0.3 for any substring match. Stacks under 1.0 cap with
            // Levenshtein component below. Empty pattern → no bonus
            // (caller validation upstream guarantees non-empty).
            let substring_bonus = if name_lc.starts_with(&needle) {
                0.5
            } else if name_lc.contains(&needle) {
                0.3
            } else {
                0.0
            };
            scored.push((substring_bonus, d, entry));
        }
    }
    // Sort: substring bonus desc (containing first), then Levenshtein asc,
    // then name asc for deterministic ties.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.name.cmp(&b.2.name))
    });
    let max_d = scored.last().map(|(_, d, _)| *d).unwrap_or(1).max(1) as f32;
    Ok(scored
        .into_iter()
        .map(|(bonus, d, mut e)| {
            // Combined score: substring bonus (0/0.3/0.5) + Levenshtein
            // component (0.0 - 0.5). Capped at 1.0. Substring matches reach
            // 1.0 only when distance is also low; non-matches max at 0.5.
            let lev_component = 0.5 - (d as f32 / max_d) * 0.5;
            e.score = (bonus + lev_component).min(1.0).max(0.0);
            e
        })
        .collect())
}

/// Spec E S-001 Contains mode. Full scan + Rust-side filter so we can
/// case-fold both sides (lbug Cypher CONTAINS is case-sensitive). Score:
/// 2.0 for prefix matches (start_with), 1.0 for mid-string. Sorted by
/// score desc, then case-insensitive name asc for deterministic ties.
fn collect_contains(conn: &lbug::Connection<'_>, pattern: &str) -> Result<Vec<SymbolEntry>> {
    let needle = pattern.to_lowercase();
    let cypher =
        "MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.kind, s.file, s.line";
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("symbols contains query: {e}")))?;

    let mut out: Vec<SymbolEntry> = Vec::new();
    for row in rs {
        let Some(mut entry) = row_to_entry(row, 1.0) else {
            continue;
        };
        let name_lc = entry.name.to_lowercase();
        if !name_lc.contains(&needle) {
            continue;
        }
        entry.score = if name_lc.starts_with(&needle) {
            2.0
        } else {
            1.0
        };
        out.push(entry);
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    Ok(out)
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
