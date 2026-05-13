//! `ga_hubs` — top-N most-connected symbols (architectural hotspots).
//!
//! Mirrors `code-review-graph::find_hub_nodes` (analysis.py:14): build per-symbol
//! degree counts from CALLS + REFERENCES edges, sort by total_degree DESC,
//! return the top-N (excluding `external` symbols).
//!
//! Intent contrast vs M2 impact retrieval:
//!   M2 work treats hubs as NOISE to filter out (EXP-009/010 — `gin.go`, etc.
//!   dominate fan-in but aren't relevant to a specific change). `ga_hubs` is
//!   the inverse intent — surface the hubs deliberately so the LLM can ask
//!   "what's the architectural backbone here?".
//!
//! Edge selection rationale (matches `code-review-graph::find_hub_nodes`
//! 2026-05-04 audit — they count `store.get_all_edges()` indiscriminately):
//!   - **CALLS**     — direct invocation.
//!   - **REFERENCES** — value-references (dispatch maps, callbacks).
//!   - **EXTENDS**   — class / trait inheritance: `B extends A` is a real
//!     "A is depended-on by B" signal even though no call site exists.
//!   - **TESTED_BY** — test coverage relationship: heavily-tested symbols
//!     are architectural anchors.
//!   - **CONTAINS**  — structural parent → child (class → method, etc.):
//!     a class with many methods accumulates degree from its members,
//!     which is exactly the "this is the architectural backbone" signal.
//!
//! Earlier opinionated subset (CALLS + REFERENCES only) was retired
//! 2026-05-04 after M3 hubs leaderboard showed near-zero / negative
//! correlation with git-churn oracle on 8 fixtures — engine was
//! measuring a strictly narrower signal than the reference impl.

use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubEntry {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub line: u32,
    pub in_degree: u32,
    pub out_degree: u32,
    pub total_degree: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubsMeta {
    pub total_symbols_with_edges: u32,
    pub truncated: bool,
    /// S-004: 1-based rank when `HubsRequest.symbol` is set. `None` in
    /// top-N mode. Computed against the FULL sorted vector (NOT truncated
    /// to `top_n`) per v1.2-Tools-C3 — a symbol ranking #51 surfaces #51
    /// even when `top_n=10`.
    #[serde(default)]
    pub target_rank: Option<u32>,
    /// S-004: `true` when `HubsRequest.symbol` matched a Symbol table row.
    /// Stays `false` (the Default) when the request was a top-N query.
    #[serde(default)]
    pub target_found: bool,
    /// S-004: top-3 Levenshtein-nearest names when `HubsRequest.symbol` was
    /// set but did not resolve. Empty in all other cases. AS-015.
    #[serde(default)]
    pub suggestion: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubsResponse {
    pub hubs: Vec<HubEntry>,
    pub meta: HubsMeta,
}

/// PR10 / AS-018 — controls whether hub degree counts the CALLS_HEURISTIC
/// subset (tier-3 repo-wide-fallback edges, low confidence). Default
/// excludes — matches Tools-C6 invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubsEdgeTypes {
    /// Exclude CALLS_HEURISTIC subset from CALLS counting. Other edge
    /// types (REFERENCES, EXTENDS, TESTED_BY, CONTAINS) unchanged.
    Default,
    /// Include every CALLS edge incl. heuristic. Opt-in for users who
    /// want maximal connectivity signal.
    All,
}

#[derive(Debug, Clone)]
pub struct HubsRequest {
    pub top_n: u32,
    /// S-004: when `Some`, switches to rank-of-target lookup mode. The
    /// returned `hubs` vec contains 0 or 1 entry (the matching one); the
    /// symbol's true rank lands in `meta.target_rank`. `top_n` is ignored
    /// in lookup mode (still bounded server-side for safety). AS-014.
    pub symbol: Option<String>,
    /// S-004: optional file disambiguator for same-name symbols across
    /// files. Ignored when `symbol` is `None`. AS-016.
    pub file: Option<String>,
    /// PR10 / AS-018 — CALLS_HEURISTIC inclusion toggle. Defaults to
    /// `Default` (exclude heuristic) per Tools-C6.
    pub edge_types: HubsEdgeTypes,
}

impl Default for HubsRequest {
    fn default() -> Self {
        Self {
            top_n: 10,
            symbol: None,
            file: None,
            edge_types: HubsEdgeTypes::Default,
        }
    }
}

/// Hard cap so a runaway `top_n` cannot DoS the response. Matches the
/// 100-result ceiling on other ga_* tools that return ranked lists.
const TOP_N_CAP: u32 = 100;

pub fn hubs(store: &Store, req: &HubsRequest) -> Result<HubsResponse> {
    let top_n = req.top_n.clamp(1, TOP_N_CAP) as usize;
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // Key = (name, file). lbug `Symbol.id` is opaque (uuid-ish per indexer);
    // (name, file) is the public-stable key the rest of the engine uses
    // (matches dead_code, callers, etc.).
    type Key = (String, String);
    let mut in_count: BTreeMap<Key, u32> = BTreeMap::new();
    let mut out_count: BTreeMap<Key, u32> = BTreeMap::new();

    // Helper inline so we can borrow into the right counter map.
    let count_into = |cypher: &str, dest: &mut BTreeMap<Key, u32>| -> Result<()> {
        let rs = conn
            .query(cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("hubs query [{cypher}]: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            let name = match &cols[0] {
                lbug::Value::String(s) if !s.is_empty() && common::is_safe_ident(s) => s.clone(),
                _ => continue,
            };
            let file = match &cols[1] {
                lbug::Value::String(s) if !s.is_empty() && !s.contains('\n') => s.clone(),
                _ => continue,
            };
            *dest.entry((name, file)).or_insert(0) += 1;
        }
        Ok(())
    };

    // Gap 7 / Fix A — count every Symbol→Symbol edge type incl v4 variants.
    // Universal-truth: hub = structural centrality measured by ALL incident
    // edges. v4 ships IMPLEMENTS / DECORATES (S→S) — pure-reuse via this list.
    // CALLS_HEURISTIC excluded (subtracted below in Default mode per Tools-C6).
    let symbol_symbol_edges = [
        "CALLS",
        "REFERENCES",
        "EXTENDS",
        "TESTED_BY",
        "CONTAINS",
        "IMPLEMENTS",
        "DECORATES",
    ];
    for kind in symbol_symbol_edges {
        count_into(
            &format!("MATCH (s:Symbol)-[:{kind}]->(t:Symbol) RETURN t.name, t.file"),
            &mut in_count,
        )?;
        count_into(
            &format!("MATCH (s:Symbol)-[:{kind}]->(t:Symbol) RETURN s.name, s.file"),
            &mut out_count,
        )?;
    }

    // Gap 7 / Fix A — count File→Symbol edges as Symbol in-degree. v4 ships
    // IMPORTS_NAMED (Symbol in-deg = importing files). v3 MODULE_TYPED
    // already exists (Symbol in-deg = module-scope type-position refs).
    // DEFINES (File→Symbol) intentionally NOT counted: every Symbol has
    // exactly 1 → no rank discrimination.
    let file_symbol_edges = ["IMPORTS_NAMED", "MODULE_TYPED"];
    for kind in file_symbol_edges {
        count_into(
            &format!("MATCH (f:File)-[:{kind}]->(t:Symbol) RETURN t.name, t.file"),
            &mut in_count,
        )?;
    }

    // PR10 / AS-018 — Default mode excludes CALLS_HEURISTIC (tier-3
    // repo-wide-fallback edges) by SUBTRACTING from the catch-all CALLS
    // counts. Tools-C7 invariant guarantees CALLS_HEURISTIC ⊆ CALLS, so
    // subtraction is sound. `edge_types: All` skips the subtraction →
    // every CALLS edge counts. Tools-C6 reference.
    if matches!(req.edge_types, HubsEdgeTypes::Default) {
        let mut heur_in: BTreeMap<Key, u32> = BTreeMap::new();
        let mut heur_out: BTreeMap<Key, u32> = BTreeMap::new();
        count_into(
            "MATCH (s:Symbol)-[:CALLS_HEURISTIC]->(t:Symbol) RETURN t.name, t.file",
            &mut heur_in,
        )?;
        count_into(
            "MATCH (s:Symbol)-[:CALLS_HEURISTIC]->(t:Symbol) RETURN s.name, s.file",
            &mut heur_out,
        )?;
        for (k, n) in heur_in {
            if let Some(v) = in_count.get_mut(&k) {
                *v = v.saturating_sub(n);
            }
        }
        for (k, n) in heur_out {
            if let Some(v) = out_count.get_mut(&k) {
                *v = v.saturating_sub(n);
            }
        }
    }

    // Union of keys with at least one edge in either direction.
    let mut seen: BTreeMap<Key, (u32, u32)> = BTreeMap::new();
    for (k, &n) in &in_count {
        seen.entry(k.clone()).or_insert((0, 0)).0 = n;
    }
    for (k, &n) in &out_count {
        seen.entry(k.clone()).or_insert((0, 0)).1 = n;
    }

    if seen.is_empty() {
        return Ok(HubsResponse::default());
    }

    // Resolve kind + line for each candidate. One Cypher per symbol would be
    // O(N) round-trips on a fixture with 1000s of symbols. Pull the whole
    // Symbol table once (excluding `external`) and join in Rust.
    let mut sym_meta: BTreeMap<Key, (String, u32)> = BTreeMap::new();
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.file, s.kind, s.line")
        .map_err(|e| Error::Other(anyhow::anyhow!("hubs symbol meta: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };
        let file = match &cols[1] {
            lbug::Value::String(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };
        let kind = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => String::new(),
        };
        let line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        sym_meta.insert((name, file), (kind, line));
    }

    let mut scored: Vec<HubEntry> = seen
        .into_iter()
        .filter_map(|(key, (in_d, out_d))| {
            // Drop symbols missing from the materialized Symbol table — those
            // are typically `external` references resolved against stdlib /
            // third-party defs we don't index. Including them would inflate
            // hub rankings with non-actionable nodes.
            let (kind, line) = sym_meta.get(&key)?.clone();
            let total = in_d.saturating_add(out_d);
            if total == 0 {
                return None;
            }
            Some(HubEntry {
                name: key.0,
                file: key.1,
                kind,
                line,
                in_degree: in_d,
                out_degree: out_d,
                total_degree: total,
            })
        })
        .collect();

    // Stable order: total DESC, then in DESC, then file ASC, then name ASC.
    // The trailing keys make the leaderboard reproducible across runs even
    // when many symbols tie on degree.
    scored.sort_by(|a, b| {
        b.total_degree
            .cmp(&a.total_degree)
            .then_with(|| b.in_degree.cmp(&a.in_degree))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.name.cmp(&b.name))
    });

    let total_with_edges = scored.len() as u32;

    // ── S-004 — symbol-lookup branch ─────────────────────────────────
    if let Some(ref target_name) = req.symbol {
        // Tools-C9-d gate: never echo non-ident bytes back into Cypher /
        // suggestion paths.
        if !common::is_safe_ident(target_name) {
            return Ok(HubsResponse::default());
        }
        let want_file = req.file.as_deref();
        // Position in the FULL sorted vec (Tools-C3) — NOT a truncated view.
        let target_pos = scored
            .iter()
            .position(|h| h.name == *target_name && want_file.is_none_or(|f| h.file == f));
        if let Some(pos) = target_pos {
            let entry = scored.swap_remove(pos);
            return Ok(HubsResponse {
                hubs: vec![entry],
                meta: HubsMeta {
                    total_symbols_with_edges: total_with_edges,
                    truncated: false,
                    target_rank: Some((pos + 1) as u32),
                    target_found: true,
                    suggestion: Vec::new(),
                },
            });
        }
        // Not found → AS-015: empty hubs + suggestion list.
        return Ok(HubsResponse {
            hubs: Vec::new(),
            meta: HubsMeta {
                total_symbols_with_edges: total_with_edges,
                truncated: false,
                target_rank: None,
                target_found: false,
                suggestion: common::suggest_similar(&conn, target_name),
            },
        });
    }

    // ── Existing top-N path ──────────────────────────────────────────
    let truncated = scored.len() > top_n;
    scored.truncate(top_n);

    Ok(HubsResponse {
        hubs: scored,
        meta: HubsMeta {
            total_symbols_with_edges: total_with_edges,
            truncated,
            target_rank: None,
            target_found: false,
            suggestion: Vec::new(),
        },
    })
}
