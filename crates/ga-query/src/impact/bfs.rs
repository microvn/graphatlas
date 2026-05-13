//! Cluster C2 — BFS over CALLS ∪ REFERENCES from a seed symbol.

use super::types::{ImpactReason, ImpactedFile};
use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use std::collections::{HashMap, HashSet};

/// EXP-M2-01 — global visited-symbol cap. Mirrors `MAX_IMPACT_NODES` at
/// `rust-poc/src/main.rs:2152`. Hub symbols with thousands of incoming
/// edges explode the visited set and blow latency + token budgets;
/// 500 is proven safe across rust-poc's production dataset.
const MAX_VISITED: usize = 500;

/// BFS over CALLS ∪ REFERENCES from `symbol` up to `max_depth` hops.
///
/// Symbol-level visited set prevents revisiting in cycles; file-level dedupe
/// keeps the min depth at which any symbol in a file surfaces. Returns
/// `(impacted_files, completeness)` where `completeness` is the deepest
/// hop actually reached during traversal (bounded by `max_depth`).
pub(super) fn bfs_from_symbol(
    store: &Store,
    symbol: &str,
    max_depth: u32,
) -> Result<(Vec<ImpactedFile>, u32, HashSet<String>)> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // (depth, reason) keyed by file path — dedupe keeps the minimum depth.
    let mut impacted: HashMap<String, (u32, ImpactReason)> = HashMap::new();
    // Symbol names already expanded — cycle guard.
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(symbol.to_string());

    // Seed files = every non-external def of `symbol`. Missing → empty
    // response (unknown symbol case).
    let seed_files = query_def_files(&conn, symbol)?;
    if seed_files.is_empty() {
        return Ok((Vec::new(), 0, HashSet::new()));
    }
    for f in seed_files {
        impacted.insert(f, (0, ImpactReason::Seed));
    }

    let mut frontier: Vec<String> = vec![symbol.to_string()];
    let mut completeness: u32 = 0;

    'outer: for depth in 1..=max_depth {
        // EXP-M2-01 — visited-set cap bounds hub-symbol explosions.
        // Check at start of depth so prior depth's work finishes
        // (matches rust-poc:2180 placement) and per-edge inside so a
        // single hub symbol with 1000+ callers doesn't blow past the cap.
        if visited.len() >= MAX_VISITED {
            break;
        }
        let mut next: Vec<String> = Vec::new();

        for name in &frontier {
            if !common::is_safe_ident(name) {
                continue; // allowlist: skip external / malformed names
            }
            // Incoming edges: who calls / references `name`.
            for (sym, file) in query_incoming(&conn, name)? {
                if visited.len() >= MAX_VISITED {
                    break 'outer;
                }
                upsert_impacted(&mut impacted, file, depth, ImpactReason::Caller);
                if visited.insert(sym.clone()) {
                    next.push(sym);
                }
            }
            // Outgoing edges: what `name` calls / references.
            for (sym, file) in query_outgoing(&conn, name)? {
                if visited.len() >= MAX_VISITED {
                    break 'outer;
                }
                upsert_impacted(&mut impacted, file, depth, ImpactReason::Callee);
                if visited.insert(sym.clone()) {
                    next.push(sym);
                }
            }
        }

        // Completeness tracks only hops that surfaced NEW symbols. Walking an
        // edge back to an already-visited node doesn't deepen the frontier.
        if next.is_empty() {
            break;
        }
        completeness = depth;
        frontier = next;
    }

    // KG-9 Action 2 — sibling-method blast radius via CONTAINS. Rust-poc
    // main.rs:2217-2227: one-shot reverse-forward Cypher that stays OUT of
    // the main BFS to prevent class explosion. Pattern:
    //   (seed)<-[:CONTAINS]-(cls)-[:CONTAINS]->(sibling)-[:CALLS]->(target)
    // Insert-if-absent at depth=2 so direct-BFS depths on overlapping
    // files are preserved. `reason: Callee` matches the semantic — we
    // discover files via a sibling's outgoing calls.
    add_sibling_method_files(&conn, symbol, &mut impacted)?;

    let mut files: Vec<ImpactedFile> = impacted
        .into_iter()
        .map(|(path, (depth, reason))| ImpactedFile {
            path,
            depth,
            reason,
            ..Default::default()
        })
        .collect();
    files.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
    Ok((files, completeness, visited))
}

/// KG-9 Action 2 — add files reachable via `seed → class → sibling → CALLS`.
/// Direct-BFS entries win on depth (we only insert when the file is absent).
fn add_sibling_method_files(
    conn: &lbug::Connection<'_>,
    seed: &str,
    impacted: &mut HashMap<String, (u32, ImpactReason)>,
) -> Result<()> {
    if !common::is_safe_ident(seed) {
        return Ok(());
    }
    let cypher = format!(
        "MATCH (s:Symbol)<-[:CONTAINS]-(cls:Symbol)-[:CONTAINS]->(sib:Symbol)\
         -[:CALLS]->(t:Symbol) \
         WHERE s.name = '{seed}' AND t.kind <> 'external' \
         RETURN DISTINCT t.file"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("sibling-method query: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::String(path)) = row.into_iter().next() {
            impacted.entry(path).or_insert((2, ImpactReason::Callee));
        }
    }
    Ok(())
}

fn upsert_impacted(
    impacted: &mut HashMap<String, (u32, ImpactReason)>,
    file: String,
    depth: u32,
    reason: ImpactReason,
) {
    match impacted.get_mut(&file) {
        Some(entry) => {
            if entry.0 > depth {
                entry.0 = depth;
                entry.1 = reason;
            }
        }
        None => {
            impacted.insert(file, (depth, reason));
        }
    }
}

fn query_def_files(conn: &lbug::Connection<'_>, name: &str) -> Result<Vec<String>> {
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{name}' AND s.kind <> 'external' \
         RETURN DISTINCT s.file"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("def-files query: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(f)) = row.into_iter().next() {
            out.push(f);
        }
    }
    Ok(out)
}

/// Callers and value-referencers of `name`, unioned. Skip external placeholders.
fn query_incoming(conn: &lbug::Connection<'_>, name: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for rel in ["CALLS", "REFERENCES"] {
        let cypher = format!(
            "MATCH (src:Symbol)-[:{rel}]->(tgt:Symbol) \
             WHERE tgt.name = '{name}' AND src.kind <> 'external' \
             RETURN src.name, src.file"
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("incoming {rel} query: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            if let (lbug::Value::String(sym), lbug::Value::String(file)) = (&cols[0], &cols[1]) {
                out.push((sym.clone(), file.clone()));
            }
        }
    }
    Ok(out)
}

/// Callees and value-references held by `name`, unioned. Skip external.
fn query_outgoing(conn: &lbug::Connection<'_>, name: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for rel in ["CALLS", "REFERENCES"] {
        let cypher = format!(
            "MATCH (src:Symbol)-[:{rel}]->(tgt:Symbol) \
             WHERE src.name = '{name}' AND tgt.kind <> 'external' \
             RETURN tgt.name, tgt.file"
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("outgoing {rel} query: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            if let (lbug::Value::String(sym), lbug::Value::String(file)) = (&cols[0], &cols[1]) {
                out.push((sym.clone(), file.clone()));
            }
        }
    }
    Ok(out)
}
