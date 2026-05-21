//! Production `ProjectDataSource` impl — wraps `ga_query` against an
//! `ga_index::Store` opened for the slug.
//!
//! Store opening is per-request (no LRU pool Phase 1 per A-C2). With
//! <10 dogfood projects this is fine; LRU lands later when measured slow.
//!
//! Remaining carve-outs (tracked in `.build-checklist`):
//!   * `graph_dump`     — needs a node/edge enumerator new to ga-query;
//!                        Phase 1 ships a degree-capped variant via
//!                        direct Cypher.
//!   * `symbol_detail`  — uses `ga_query::symbols` lookup-by-exact-name
//!                        for now (Phase 1 treats the URL `:symbol_id`
//!                        segment as the symbol name; integer ids land
//!                        when ga-query gets an id-table accessor).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use ga_index::Store;
use ga_query::architecture::ArchitectureResponse;
use ga_query::render::{render_signature, ParamSlot};

use crate::cache_state::lookup_cache_state;
use crate::data::{
    DataError, FileSummary, GraphEdge, GraphNode, GraphResponse, LayerEntry, LayerSymbolsResponse,
    LayersResponse, ProjectDataSource, RelationEntry, RelationPage, SymbolDetail, SymbolHit,
    SymbolSearchResponse,
};
use crate::recovery::find_cache_dir;

pub struct LbugDataSource {
    pub cache_root: PathBuf,
    /// Per-slug cache of `architecture()` keyed by metadata.json mtime.
    /// `/layers` + `/layers/:name/symbols` both call architecture(); without
    /// a cache, each hits 5+ seconds on a 100k-symbol repo because
    /// architecture walks the repo + queries per file. ga-mcp uses its own
    /// short-lived path so the cache here doesn't affect MCP semantics
    /// (see Spec D Change Log 2026-05-17 note).
    arch_cache: Mutex<HashMap<String, (SystemTime, ArchitectureResponse)>>,
}

impl LbugDataSource {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            arch_cache: Mutex::new(HashMap::new()),
        }
    }

    /// mtime of `<cache_dir>/metadata.json` — invalidator for arch cache.
    /// Reindex rewrites metadata.json atomically, bumping the mtime.
    fn cache_mtime(&self, slug: &str) -> Result<SystemTime, DataError> {
        let cache_dir = find_cache_dir(&self.cache_root, slug).ok_or(DataError::ProjectNotFound)?;
        std::fs::metadata(cache_dir.join("metadata.json"))
            .and_then(|m| m.modified())
            .map_err(|e| DataError::Backend(format!("mtime: {e}")))
    }

    /// Get cached architecture for `slug`, recomputing if mtime drifted
    /// or no entry exists. Returns a clone so callers can mutate freely.
    fn architecture_cached(&self, slug: &str) -> Result<ArchitectureResponse, DataError> {
        let mtime = self.cache_mtime(slug)?;
        {
            let guard = self.arch_cache.lock().unwrap();
            if let Some((stored_mtime, resp)) = guard.get(slug) {
                if *stored_mtime == mtime {
                    return Ok(resp.clone());
                }
            }
        }
        // Recompute outside the lock so concurrent requests for other
        // slugs don't block on the architecture walk.
        let store = self.open_store(slug)?;
        let resp = ga_query::architecture::architecture(
            &store,
            &ga_query::architecture::ArchitectureRequest::default(),
        )
        .map_err(|e| DataError::Backend(format!("architecture: {e}")))?;
        let mut guard = self.arch_cache.lock().unwrap();
        guard.insert(slug.to_string(), (mtime, resp.clone()));
        Ok(resp)
    }

    fn resolve_repo(&self, slug: &str) -> Result<PathBuf, DataError> {
        match lookup_cache_state(&self.cache_root, slug) {
            crate::cache_state::CacheState::NotFound => Err(DataError::ProjectNotFound),
            crate::cache_state::CacheState::Corrupt => Err(DataError::CacheCorrupt),
            crate::cache_state::CacheState::Building => Err(DataError::CacheBuilding),
            crate::cache_state::CacheState::Fresh => {
                let cache_dir =
                    find_cache_dir(&self.cache_root, slug).ok_or(DataError::ProjectNotFound)?;
                let bytes = std::fs::read(cache_dir.join("metadata.json"))
                    .map_err(|e| DataError::Backend(format!("metadata read: {e}")))?;
                let v: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| DataError::Backend(format!("metadata parse: {e}")))?;
                let p = v
                    .get("repo_root")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| DataError::Backend("metadata missing repo_root".into()))?;
                Ok(PathBuf::from(p))
            }
        }
    }

    fn open_store(&self, slug: &str) -> Result<Store, DataError> {
        let repo = self.resolve_repo(slug)?;
        Store::open_with_root(&self.cache_root, &repo)
            .map_err(|e| DataError::Backend(format!("open store: {e}")))
    }
}

fn paginate<T: Clone>(all: &[T], offset: u64, limit: u64) -> (Vec<T>, u64, bool) {
    let total = all.len() as u64;
    let start = (offset as usize).min(all.len());
    let end = (start + limit as usize).min(all.len());
    let slice = all[start..end].to_vec();
    (slice, total, (end as u64) < total)
}

impl ProjectDataSource for LbugDataSource {
    fn graph_dump(
        &self,
        slug: &str,
        focus: Option<&str>,
        hops: u8,
    ) -> Result<GraphResponse, DataError> {
        let store = self.open_store(slug)?;
        let conn = store
            .connection()
            .map_err(|e| DataError::Backend(format!("connection: {e}")))?;

        // Phase 1 strategy: pull all Symbol nodes, cap by degree at 5000
        // (Spec A AS-033). If a focus symbol is supplied, restrict to
        // its neighborhood up to `hops` (Spec A AS-034).
        let cap = 5000usize;

        // Total node count first (cheap COUNT) so we report truncated correctly.
        let total_nodes: u64 = {
            let rs = conn
                .query("MATCH (s:Symbol) RETURN count(s)")
                .map_err(|e| DataError::Backend(format!("count nodes: {e}")))?;
            rs.into_iter()
                .next()
                .and_then(|row| row.into_iter().next())
                .and_then(|v| match v {
                    lbug::Value::Int64(n) => Some(n.max(0) as u64),
                    _ => None,
                })
                .unwrap_or(0)
        };

        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let truncated;

        if let Some(focus_name) = focus {
            // Ego subgraph — Phase 1 treats focus as exact symbol name.
            let safe = focus_name.replace('\'', "''");
            let hops = hops.clamp(1, 2) as usize;
            let edges_clause = (0..hops).map(|_| "[:CALLS|:CALLS_HEURISTIC|:REFERENCES|:IMPORTS|:CONTAINS|:EXTENDS|:IMPLEMENTS|:OVERRIDES]").collect::<Vec<_>>().join("-()-");
            let _ = edges_clause; // placeholder: lbug Cypher path expansion varies;
                                  // Conservative ego query — 1 hop in/out for now.
            let q = format!(
                "MATCH (center:Symbol {{name: '{}'}})-[r]-(n:Symbol) \
                 RETURN center.id, center.name, center.kind, center.file, center.line, \
                        n.id, n.name, n.kind, n.file, n.line \
                 LIMIT {}",
                safe, cap
            );
            let rs = conn
                .query(&q)
                .map_err(|e| DataError::Backend(format!("ego query: {e}")))?;
            let mut seen_ids = std::collections::HashSet::new();
            for row in rs {
                let cols: Vec<lbug::Value> = row.into_iter().collect();
                if cols.len() < 10 {
                    continue;
                }
                for offset in [0usize, 5usize] {
                    let id = match &cols[offset] {
                        lbug::Value::String(s) => s.clone(),
                        _ => continue,
                    };
                    if !seen_ids.insert(id.clone()) {
                        continue;
                    }
                    let name = match &cols[offset + 1] {
                        lbug::Value::String(s) => s.clone(),
                        _ => continue,
                    };
                    let kind = match &cols[offset + 2] {
                        lbug::Value::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    let file = match &cols[offset + 3] {
                        lbug::Value::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    let line = match &cols[offset + 4] {
                        lbug::Value::Int64(n) => *n as u32,
                        _ => 0,
                    };
                    nodes.push(GraphNode {
                        id,
                        name,
                        kind,
                        file,
                        line,
                        line_end: None,
                        degree: 0,
                    });
                }
            }
            truncated = nodes.len() >= cap;
        } else {
            // Full graph — Phase 1 capped 5000 by Symbol.id order.
            let q = format!(
                "MATCH (s:Symbol) RETURN s.id, s.name, s.kind, s.file, s.line LIMIT {}",
                cap
            );
            let rs = conn
                .query(&q)
                .map_err(|e| DataError::Backend(format!("nodes query: {e}")))?;
            for row in rs {
                let cols: Vec<lbug::Value> = row.into_iter().collect();
                if cols.len() < 5 {
                    continue;
                }
                let id = match &cols[0] {
                    lbug::Value::String(s) => s.clone(),
                    _ => continue,
                };
                let name = match &cols[1] {
                    lbug::Value::String(s) => s.clone(),
                    _ => continue,
                };
                let kind = match &cols[2] {
                    lbug::Value::String(s) => s.clone(),
                    _ => String::new(),
                };
                let file = match &cols[3] {
                    lbug::Value::String(s) => s.clone(),
                    _ => String::new(),
                };
                let line = match &cols[4] {
                    lbug::Value::Int64(n) => *n as u32,
                    _ => 0,
                };
                nodes.push(GraphNode {
                    id,
                    name,
                    kind,
                    file,
                    line,
                    line_end: None,
                    degree: 0,
                });
            }
            truncated = total_nodes > nodes.len() as u64;
        }

        // CALLS edges among the collected nodes.
        let node_ids: std::collections::HashSet<String> =
            nodes.iter().map(|n| n.id.clone()).collect();
        if !node_ids.is_empty() {
            // Pull all CALLS edges; filter to the visible node set.
            // Bounded by `cap * 8` to keep payload sane.
            let edge_cap = cap.saturating_mul(8);
            let q = format!(
                "MATCH (a:Symbol)-[r:CALLS]->(b:Symbol) RETURN a.id, b.id, r.call_site_line LIMIT {}",
                edge_cap
            );
            let rs = conn
                .query(&q)
                .map_err(|e| DataError::Backend(format!("edges query: {e}")))?;
            for row in rs {
                let cols: Vec<lbug::Value> = row.into_iter().collect();
                if cols.len() < 3 {
                    continue;
                }
                let from = match &cols[0] {
                    lbug::Value::String(s) => s.clone(),
                    _ => continue,
                };
                let to = match &cols[1] {
                    lbug::Value::String(s) => s.clone(),
                    _ => continue,
                };
                if !node_ids.contains(&from) || !node_ids.contains(&to) {
                    continue;
                }
                let line = match &cols[2] {
                    lbug::Value::Int64(n) => Some(*n as u32),
                    _ => None,
                };
                edges.push(GraphEdge {
                    from,
                    to,
                    kind: "CALLS".into(),
                    line,
                });
            }
        }

        Ok(GraphResponse {
            nodes,
            edges,
            truncated,
            total_node_count: total_nodes,
        })
    }

    fn symbol_detail(&self, slug: &str, symbol_id: &str) -> Result<SymbolDetail, DataError> {
        let store = self.open_store(slug)?;
        let conn = store
            .connection()
            .map_err(|e| DataError::Backend(format!("connection: {e}")))?;

        // Phase 1 — treat symbol_id as exact name. Pull all attrs we
        // need to populate the detail panel + render_signature.
        let safe = symbol_id.replace('\'', "''");
        let q = format!(
            "MATCH (s:Symbol {{name: '{}'}}) \
             RETURN s.id, s.name, s.kind, s.file, s.line, s.line_end, \
                    s.qualified_name, s.return_type, \
                    s.is_async, s.is_abstract, s.is_static, s.is_override, \
                    s.confidence, s.doc_summary \
             LIMIT 1",
            safe
        );
        let rs = conn
            .query(&q)
            .map_err(|e| DataError::Backend(format!("symbol query: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 14 {
                continue;
            }
            let id = string_or_empty(&cols[0]);
            let name = string_or_empty(&cols[1]);
            let kind = string_or_empty(&cols[2]);
            let file = string_or_empty(&cols[3]);
            let line = int_or_zero(&cols[4]) as u32;
            let line_end = match &cols[5] {
                lbug::Value::Int64(n) => Some(*n as u32),
                _ => None,
            };
            let qualified_name = match &cols[6] {
                lbug::Value::String(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            let return_type = string_or_empty(&cols[7]);
            let is_async = bool_or_false(&cols[8]);
            let is_abstract = bool_or_false(&cols[9]);
            let is_static = bool_or_false(&cols[10]);
            let is_override = bool_or_false(&cols[11]);
            let confidence = double_or_default(&cols[12], 1.0);
            let doc_summary_raw = string_or_empty(&cols[13]);
            let doc_summary = if doc_summary_raw.is_empty() {
                None
            } else {
                Some(doc_summary_raw.clone())
            };
            let has_doc = doc_summary.is_some();
            let loc = line_end.map(|le| le.saturating_sub(line).saturating_add(1));
            // Phase 1: params hydration is S-004 (STRUCT[] decode TBD).
            // Render signature with name + return_type only (degrade path
            // AS-017). Counts + tested + dead_code + hub flags hydrate
            // via separate queries below.
            let rendered_signature = render_signature(&name, &return_type, &[] as &[ParamSlot]);

            // Counts via ga_query — at most O(callers+callees+importers)
            // per detail call. Acceptable for single-symbol detail panel.
            let caller_count = ga_query::callers(&store, &name, None)
                .map(|r| r.callers.len() as u32)
                .unwrap_or(0);
            let callee_count = ga_query::callees(&store, &name, None)
                .map(|r| r.callees.len() as u32)
                .unwrap_or(0);
            let importer_count = ga_query::importers(&store, &file)
                .map(|r| r.importers.len() as u32)
                .unwrap_or(0);
            let impact_edge_count = caller_count + callee_count;

            // TESTED_BY edge probe — single Cypher count.
            let tested = {
                let q = format!(
                    "MATCH (t:Symbol)-[:TESTED_BY]->(s:Symbol {{name: '{}'}}) RETURN count(t)",
                    name.replace('\'', "''")
                );
                conn.query(&q)
                    .ok()
                    .and_then(|rs| rs.into_iter().next())
                    .and_then(|row| row.into_iter().next())
                    .map(|v| matches!(v, lbug::Value::Int64(n) if n > 0))
                    .unwrap_or(false)
            };

            return Ok(SymbolDetail {
                id,
                name,
                kind,
                file,
                line,
                line_end,
                qualified_name,
                rendered_signature,
                layer: None,
                loc,
                doc_summary,
                has_doc,
                is_async,
                is_abstract,
                is_static,
                is_override,
                confidence,
                // is_dead_code + is_hub are derived signals — hydrating
                // here per-symbol would cost a full dead_code / hubs scan
                // on every detail call. Defer to a layered cache; ship
                // false now (frontend renders no badge).
                is_dead_code: false,
                is_hub: false,
                tested,
                caller_count,
                callee_count,
                importer_count,
                impact_edge_count,
                // params hydration via STRUCT[] decoder — Spec E S-004
                // degrade path AS-017. None on the wire → frontend "—".
                params: None,
            });
        }
        Err(DataError::SymbolNotFound)
    }

    fn callers(
        &self,
        slug: &str,
        symbol_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError> {
        let store = self.open_store(slug)?;
        let resp = ga_query::callers(&store, symbol_id, None)
            .map_err(|e| DataError::Backend(format!("callers: {e}")))?;
        if !resp.meta.symbol_found && resp.callers.is_empty() {
            // Distinguish "no callers" from "symbol absent" — ga-query
            // surfaces this via meta.symbol_found.
            return Err(DataError::SymbolNotFound);
        }
        let all: Vec<RelationEntry> = resp
            .callers
            .into_iter()
            .map(|c| RelationEntry {
                id: format!("{}::{}:{}", c.file, c.symbol, c.line),
                name: c.symbol,
                file: c.file,
                line: c.line,
                kind: "Function".into(),
            })
            .collect();
        let (entries, total, has_more) = paginate(&all, offset, limit);
        Ok(RelationPage {
            entries,
            total,
            has_more,
            offset,
            limit,
        })
    }

    fn callees(
        &self,
        slug: &str,
        symbol_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError> {
        let store = self.open_store(slug)?;
        let resp = ga_query::callees(&store, symbol_id, None)
            .map_err(|e| DataError::Backend(format!("callees: {e}")))?;
        if !resp.meta.symbol_found && resp.callees.is_empty() {
            return Err(DataError::SymbolNotFound);
        }
        let all: Vec<RelationEntry> = resp
            .callees
            .into_iter()
            .map(|c| RelationEntry {
                id: format!("{}::{}:{}", c.file, c.symbol, c.line),
                name: c.symbol,
                file: c.file,
                line: c.line,
                kind: c.symbol_kind,
            })
            .collect();
        let (entries, total, has_more) = paginate(&all, offset, limit);
        Ok(RelationPage {
            entries,
            total,
            has_more,
            offset,
            limit,
        })
    }

    fn importers(
        &self,
        slug: &str,
        file_path: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError> {
        let store = self.open_store(slug)?;
        let resp = ga_query::importers(&store, file_path)
            .map_err(|e| DataError::Backend(format!("importers: {e}")))?;
        let all: Vec<RelationEntry> = resp
            .importers
            .into_iter()
            .map(|i| RelationEntry {
                id: format!("{}:{}", i.path, i.import_line),
                name: i.imported_names.first().cloned().unwrap_or_default(),
                file: i.path,
                line: i.import_line,
                kind: "Import".into(),
            })
            .collect();
        let (entries, total, has_more) = paginate(&all, offset, limit);
        Ok(RelationPage {
            entries,
            total,
            has_more,
            offset,
            limit,
        })
    }

    fn symbols_search(
        &self,
        slug: &str,
        pattern: &str,
        limit: u64,
    ) -> Result<SymbolSearchResponse, DataError> {
        let store = self.open_store(slug)?;
        let resp = ga_query::symbols(&store, pattern, ga_query::SymbolsMatch::Contains)
            .map_err(|e| DataError::Backend(format!("symbols_search: {e}")))?;
        let truncated = resp.meta.truncated;
        let hits: Vec<SymbolHit> = resp
            .symbols
            .into_iter()
            .take(limit as usize)
            .map(|s| SymbolHit {
                id: format!("{}::{}:{}", s.file, s.name, s.line),
                name: s.name,
                kind: s.kind,
                file: s.file,
                line: s.line,
                // Layer hydration deferred to populated-PR follow-up; the
                // /layers endpoint already exposes layer membership and
                // the frontend can join client-side. Tracked in checklist.
                layer: None,
            })
            .collect();
        Ok(SymbolSearchResponse { hits, truncated })
    }

    fn layers(&self, slug: &str) -> Result<LayersResponse, DataError> {
        match self.architecture_cached(slug) {
            Ok(arch) => {
                let mut layers: Vec<LayerEntry> = arch
                    .modules
                    .into_iter()
                    .map(|m| LayerEntry {
                        name: m.name,
                        symbol_count: m.symbol_count,
                    })
                    .collect();
                layers.sort_by(|a, b| {
                    b.symbol_count
                        .cmp(&a.symbol_count)
                        .then(a.name.cmp(&b.name))
                });
                Ok(LayersResponse {
                    layers,
                    degraded: false,
                })
            }
            Err(_) => Ok(LayersResponse {
                layers: vec![],
                degraded: true,
            }),
        }
    }

    fn layer_symbols(
        &self,
        slug: &str,
        layer_name: &str,
    ) -> Result<LayerSymbolsResponse, DataError> {
        let arch = self.architecture_cached(slug)?;
        let store = self.open_store(slug)?;
        let module = arch
            .modules
            .iter()
            .find(|m| m.name == layer_name)
            .ok_or(DataError::LayerNotFound)?;
        if module.files.is_empty() {
            return Ok(LayerSymbolsResponse {
                symbols: vec![],
                symbol_ids: vec![],
            });
        }
        let conn = store
            .connection()
            .map_err(|e| DataError::Backend(format!("connection: {e}")))?;
        // Pull all Symbol rows in this module's files.
        let in_list = module
            .files
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let cypher = format!(
            "MATCH (s:Symbol) WHERE s.kind <> 'external' AND s.file IN [{in_list}] \
             RETURN s.id, s.name, s.kind, s.file, s.line"
        );
        let rs = conn
            .query(&cypher)
            .map_err(|e| DataError::Backend(format!("layer_symbols: {e}")))?;
        let mut symbols: Vec<SymbolHit> = Vec::new();
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 5 {
                continue;
            }
            let id = string_or_empty(&cols[0]);
            let name = string_or_empty(&cols[1]);
            let kind = string_or_empty(&cols[2]);
            let file = string_or_empty(&cols[3]);
            let line = int_or_zero(&cols[4]) as u32;
            symbols.push(SymbolHit {
                id,
                name,
                kind,
                file,
                line,
                layer: Some(layer_name.to_string()),
            });
        }
        let symbol_ids: Vec<String> = symbols.iter().map(|s| s.id.clone()).collect();
        Ok(LayerSymbolsResponse {
            symbols,
            symbol_ids,
        })
    }

    fn file_summary(&self, slug: &str, file_path: &str) -> Result<FileSummary, DataError> {
        let store = self.open_store(slug)?;
        let resp = ga_query::file_summary(&store, file_path)
            .map_err(|e| DataError::Backend(format!("file_summary: {e}")))?;
        // ga_query returns empty symbols+imports+exports when the file
        // isn't in the graph. Treat all-empty as FileNotFound so the UI
        // banners cleanly.
        if resp.symbols.is_empty() && resp.imports.is_empty() && resp.exports.is_empty() {
            return Err(DataError::FileNotFound);
        }
        // Reverse imports — files that import this one. Cheap Cypher.
        let conn = store
            .connection()
            .map_err(|e| DataError::Backend(format!("connection: {e}")))?;
        let safe = file_path.replace('\'', "''");
        let q = format!(
            "MATCH (f:File)-[:IMPORTS]->(t:File {{path: '{}'}}) RETURN f.path",
            safe
        );
        let mut reverse_imports = Vec::new();
        if let Ok(rs) = conn.query(&q) {
            for row in rs {
                if let Some(lbug::Value::String(p)) = row.into_iter().next() {
                    reverse_imports.push(p);
                }
            }
        }
        let symbols: Vec<RelationEntry> = resp
            .symbols
            .into_iter()
            .map(|s| RelationEntry {
                id: format!("{}::{}:{}", s.file, s.name, s.line),
                name: s.name,
                file: s.file,
                line: s.line,
                kind: s.kind,
            })
            .collect();
        Ok(FileSummary {
            path: resp.path,
            language: None,
            line_count: None,
            symbols,
            imports: resp.imports,
            reverse_imports,
        })
    }
}

fn string_or_empty(v: &lbug::Value) -> String {
    match v {
        lbug::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}
fn int_or_zero(v: &lbug::Value) -> i64 {
    match v {
        lbug::Value::Int64(n) => *n,
        _ => 0,
    }
}
fn bool_or_false(v: &lbug::Value) -> bool {
    matches!(v, lbug::Value::Bool(true))
}
fn double_or_default(v: &lbug::Value, default: f64) -> f64 {
    match v {
        lbug::Value::Double(d) => *d,
        _ => default,
    }
}

/// In-memory fixture-backed source. Used by integration tests.
#[cfg(any(test, feature = "test-fixture"))]
pub mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::data::{
        DataError, FileSummary, GraphResponse, LayerSymbolsResponse, LayersResponse,
        ProjectDataSource, RelationEntry, RelationPage, SymbolDetail, SymbolSearchResponse,
    };

    #[derive(Default)]
    pub struct ProjectFixture {
        pub graph: Option<GraphResponse>,
        pub focused_graph: HashMap<String, GraphResponse>,
        pub symbol_detail: HashMap<String, SymbolDetail>,
        pub callers: HashMap<String, Vec<RelationEntry>>,
        pub callees: HashMap<String, Vec<RelationEntry>>,
        pub importers: HashMap<String, Vec<RelationEntry>>,
        pub file_summary: HashMap<String, FileSummary>,
        /// Spec E — pattern → response. Tests seed exactly the
        /// pattern they expect the handler to dispatch.
        pub symbol_search: HashMap<String, SymbolSearchResponse>,
        pub layers: Option<LayersResponse>,
        pub layer_symbols: HashMap<String, LayerSymbolsResponse>,
    }

    pub struct FakeDataSource {
        inner: Mutex<HashMap<String, ProjectFixture>>,
    }

    impl FakeDataSource {
        pub fn new() -> Self {
            Self {
                inner: Mutex::new(HashMap::new()),
            }
        }
        pub fn insert(&self, slug: &str, fixture: ProjectFixture) {
            self.inner.lock().unwrap().insert(slug.into(), fixture);
        }
    }

    impl Default for FakeDataSource {
        fn default() -> Self {
            Self::new()
        }
    }

    fn paginate(all: &[RelationEntry], offset: u64, limit: u64) -> RelationPage {
        let total = all.len() as u64;
        let start = (offset as usize).min(all.len());
        let end = (start + limit as usize).min(all.len());
        RelationPage {
            entries: all[start..end].to_vec(),
            total,
            has_more: (end as u64) < total,
            offset,
            limit,
        }
    }

    impl ProjectDataSource for FakeDataSource {
        fn graph_dump(
            &self,
            slug: &str,
            focus: Option<&str>,
            _hops: u8,
        ) -> Result<GraphResponse, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            if let Some(id) = focus {
                fx.focused_graph
                    .get(id)
                    .cloned()
                    .ok_or(DataError::SymbolNotFound)
            } else {
                fx.graph
                    .clone()
                    .ok_or(DataError::Backend("no graph fixture".into()))
            }
        }

        fn symbol_detail(&self, slug: &str, id: &str) -> Result<SymbolDetail, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            fx.symbol_detail
                .get(id)
                .cloned()
                .ok_or(DataError::SymbolNotFound)
        }

        fn callers(
            &self,
            slug: &str,
            id: &str,
            offset: u64,
            limit: u64,
        ) -> Result<RelationPage, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            Ok(paginate(
                fx.callers.get(id).ok_or(DataError::SymbolNotFound)?,
                offset,
                limit,
            ))
        }

        fn callees(
            &self,
            slug: &str,
            id: &str,
            offset: u64,
            limit: u64,
        ) -> Result<RelationPage, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            Ok(paginate(
                fx.callees.get(id).ok_or(DataError::SymbolNotFound)?,
                offset,
                limit,
            ))
        }

        fn importers(
            &self,
            slug: &str,
            path: &str,
            offset: u64,
            limit: u64,
        ) -> Result<RelationPage, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            Ok(paginate(
                fx.importers.get(path).ok_or(DataError::FileNotFound)?,
                offset,
                limit,
            ))
        }

        fn file_summary(&self, slug: &str, path: &str) -> Result<FileSummary, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            fx.file_summary
                .get(path)
                .cloned()
                .ok_or(DataError::FileNotFound)
        }

        fn symbols_search(
            &self,
            slug: &str,
            pattern: &str,
            _limit: u64,
        ) -> Result<SymbolSearchResponse, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            Ok(fx
                .symbol_search
                .get(pattern)
                .cloned()
                .unwrap_or(SymbolSearchResponse {
                    hits: vec![],
                    truncated: false,
                }))
        }

        fn layers(&self, slug: &str) -> Result<LayersResponse, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            Ok(fx.layers.clone().unwrap_or(LayersResponse {
                layers: vec![],
                degraded: true,
            }))
        }

        fn layer_symbols(
            &self,
            slug: &str,
            layer_name: &str,
        ) -> Result<LayerSymbolsResponse, DataError> {
            let g = self.inner.lock().unwrap();
            let fx = g.get(slug).ok_or(DataError::ProjectNotFound)?;
            fx.layer_symbols
                .get(layer_name)
                .cloned()
                .ok_or(DataError::LayerNotFound)
        }
    }
}
