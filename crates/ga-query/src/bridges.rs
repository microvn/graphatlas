//! `ga_bridges` — top-N architectural chokepoints via betweenness centrality.
//!
//! Mirrors `code-review-graph::find_bridge_nodes` (analysis.py:58). Bridge
//! nodes sit on shortest paths between many pairs; if they break, multiple
//! code regions lose connectivity. Different intent from `ga_hubs`:
//!   - Hub  = high direct in/out degree (lots of immediate callers/callees)
//!   - Bridge = high mediating role (control point in the topology)
//!
//! ## Algorithm
//! Brandes' algorithm — O(V·E) on unweighted graphs. Implemented directly
//! in ~120 lines so we don't pull a heavy graph crate (`rustworkx-core`,
//! `petgraph` algos crate) for one query path. Sampling source vertices
//! when V > 5000 (k = 500, scaled by V/k) per the same heuristic
//! code-review-graph uses.
//!
//! ## Edge selection
//! CALLS + REFERENCES, treated as undirected for centrality (a bridge is
//! a chokepoint regardless of edge direction). External symbols excluded
//! before the BFS so they cannot inflate path counts.

use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeEntry {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub line: u32,
    /// Normalized betweenness centrality, in `[0, 1]`. 0 = never on a
    /// shortest path. Higher = more critical chokepoint.
    pub betweenness: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgesMeta {
    pub total_nodes: u32,
    pub total_edges: u32,
    /// `true` when source-vertex sampling was used (V > 5000). The reported
    /// betweenness is then a Monte-Carlo approximation, not an exact value.
    pub sampled: bool,
    pub sample_size: u32,
    pub truncated: bool,
    /// S-004: 1-based rank when `BridgesRequest.symbol` is set. Mirrors
    /// `HubsMeta.target_rank`. Computed against the FULL post-Brandes
    /// vec (Tools-C3) — not the truncated top-N view.
    #[serde(default)]
    pub target_rank: Option<u32>,
    #[serde(default)]
    pub target_found: bool,
    #[serde(default)]
    pub suggestion: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgesResponse {
    pub bridges: Vec<BridgeEntry>,
    pub meta: BridgesMeta,
}

#[derive(Debug, Clone)]
pub struct BridgesRequest {
    pub top_n: u32,
    /// S-004: when `Some`, rank-of-target lookup mode. AS-017.
    pub symbol: Option<String>,
    /// S-004: optional file disambiguator.
    pub file: Option<String>,
}

impl Default for BridgesRequest {
    fn default() -> Self {
        Self {
            top_n: 10,
            symbol: None,
            file: None,
        }
    }
}

const TOP_N_CAP: u32 = 100;
const SAMPLE_THRESHOLD: usize = 5000;
const SAMPLE_SIZE: usize = 500;

pub fn bridges(store: &Store, req: &BridgesRequest) -> Result<BridgesResponse> {
    let top_n = req.top_n.clamp(1, TOP_N_CAP) as usize;
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // ─── Pull non-external symbols + meta ───
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.file, s.kind, s.line")
        .map_err(|e| Error::Other(anyhow::anyhow!("bridges symbol meta: {e}")))?;

    type Key = (String, String);
    let mut nodes: Vec<(Key, String, u32)> = Vec::new();
    let mut idx: HashMap<Key, u32> = HashMap::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
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
        let kind = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => String::new(),
        };
        let line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let key = (name, file);
        if !idx.contains_key(&key) {
            idx.insert(key.clone(), nodes.len() as u32);
            nodes.push((key, kind, line));
        }
    }

    let n = nodes.len();
    if n == 0 {
        return Ok(BridgesResponse::default());
    }

    // ─── Build undirected adjacency from CALLS + REFERENCES ───
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut total_edges: u64 = 0;

    let load_edges = |cypher: &str, adj: &mut Vec<Vec<u32>>, total: &mut u64| -> Result<()> {
        let rs = conn
            .query(cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("bridges edges [{cypher}]: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 4 {
                continue;
            }
            let sn = match &cols[0] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let sf = match &cols[1] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let tn = match &cols[2] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let tf = match &cols[3] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let (Some(&u), Some(&v)) = (idx.get(&(sn, sf)), idx.get(&(tn, tf))) else {
                continue;
            };
            if u == v {
                continue;
            }
            adj[u as usize].push(v);
            adj[v as usize].push(u);
            *total += 1;
        }
        Ok(())
    };

    load_edges(
        "MATCH (s:Symbol)-[:CALLS]->(t:Symbol) RETURN s.name, s.file, t.name, t.file",
        &mut adj,
        &mut total_edges,
    )?;
    load_edges(
        "MATCH (s:Symbol)-[:REFERENCES]->(t:Symbol) RETURN s.name, s.file, t.name, t.file",
        &mut adj,
        &mut total_edges,
    )?;

    // Dedup adjacency (multi-edges between same pair → single neighbour).
    for list in adj.iter_mut() {
        list.sort_unstable();
        list.dedup();
    }

    // ─── Brandes' betweenness ───
    let (sources, sampled) = if n > SAMPLE_THRESHOLD {
        // Deterministic sampling: stride n / SAMPLE_SIZE so the sample is
        // reproducible across runs (the same lock-file gating applies to
        // bench scoring — we want the same nodes picked every time).
        let stride = (n / SAMPLE_SIZE).max(1);
        let picked: Vec<u32> = (0..n)
            .step_by(stride)
            .take(SAMPLE_SIZE)
            .map(|i| i as u32)
            .collect();
        (picked, true)
    } else {
        ((0..n as u32).collect(), false)
    };
    let sample_size = sources.len();

    let mut bc = vec![0.0_f64; n];
    let mut sigma = vec![0.0_f64; n];
    let mut dist = vec![-1_i32; n];
    let mut delta = vec![0.0_f64; n];
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];

    for &s in &sources {
        // Reset per-source scratch.
        for v in 0..n {
            sigma[v] = 0.0;
            dist[v] = -1;
            delta[v] = 0.0;
            preds[v].clear();
        }
        sigma[s as usize] = 1.0;
        dist[s as usize] = 0;

        let mut stack: Vec<u32> = Vec::new();
        let mut q: VecDeque<u32> = VecDeque::new();
        q.push_back(s);
        while let Some(v) = q.pop_front() {
            stack.push(v);
            let dv = dist[v as usize];
            let sv = sigma[v as usize];
            for &w in &adj[v as usize] {
                if dist[w as usize] < 0 {
                    dist[w as usize] = dv + 1;
                    q.push_back(w);
                }
                if dist[w as usize] == dv + 1 {
                    sigma[w as usize] += sv;
                    preds[w as usize].push(v);
                }
            }
        }

        while let Some(w) = stack.pop() {
            let sw = sigma[w as usize];
            let dw_term = 1.0 + delta[w as usize];
            for &v in &preds[w as usize] {
                if sw > 0.0 {
                    delta[v as usize] += (sigma[v as usize] / sw) * dw_term;
                }
            }
            if w != s {
                bc[w as usize] += delta[w as usize];
            }
        }
    }

    // Normalize: undirected → divide by 2; sampling → scale by N/k; pair
    // count for normalisation = (n-1)(n-2) for directed; halve for undirected.
    let pair_count = ((n.saturating_sub(1)) as f64) * ((n.saturating_sub(2)) as f64);
    let scale_sample = if sampled {
        (n as f64) / (sample_size as f64)
    } else {
        1.0
    };
    let denom = if pair_count > 0.0 { pair_count } else { 1.0 };
    for v in 0..n {
        bc[v] = (bc[v] / 2.0) * scale_sample / denom;
    }

    // ─── Sort + emit top_n ───
    let mut scored: Vec<BridgeEntry> = (0..n)
        .filter(|&i| bc[i] > 0.0)
        .map(|i| {
            let (key, kind, line) = &nodes[i];
            BridgeEntry {
                name: key.0.clone(),
                file: key.1.clone(),
                kind: kind.clone(),
                line: *line,
                betweenness: bc[i],
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.betweenness
            .partial_cmp(&a.betweenness)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.name.cmp(&b.name))
    });

    // ── S-004 — symbol-lookup branch (mirror of hubs.rs) ─────────────
    if let Some(ref target_name) = req.symbol {
        if !common::is_safe_ident(target_name) {
            return Ok(BridgesResponse::default());
        }
        let want_file = req.file.as_deref();
        let target_pos = scored
            .iter()
            .position(|b| b.name == *target_name && want_file.is_none_or(|f| b.file == f));
        if let Some(pos) = target_pos {
            let entry = scored.swap_remove(pos);
            return Ok(BridgesResponse {
                bridges: vec![entry],
                meta: BridgesMeta {
                    total_nodes: n as u32,
                    total_edges: total_edges as u32,
                    sampled,
                    sample_size: sample_size as u32,
                    truncated: false,
                    target_rank: Some((pos + 1) as u32),
                    target_found: true,
                    suggestion: Vec::new(),
                },
            });
        }
        return Ok(BridgesResponse {
            bridges: Vec::new(),
            meta: BridgesMeta {
                total_nodes: n as u32,
                total_edges: total_edges as u32,
                sampled,
                sample_size: sample_size as u32,
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

    Ok(BridgesResponse {
        bridges: scored,
        meta: BridgesMeta {
            total_nodes: n as u32,
            total_edges: total_edges as u32,
            sampled,
            sample_size: sample_size as u32,
            truncated,
            target_rank: None,
            target_found: false,
            suggestion: Vec::new(),
        },
    })
}
