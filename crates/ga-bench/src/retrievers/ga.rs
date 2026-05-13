//! In-process GraphAtlas retriever. Builds the `Store` + runs `build_index`
//! in `setup`, then dispatches each task to the matching `ga_query::*` entry
//! point. No subprocess — this is the "fast path" and the retriever we own.

use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use ga_index::Store;
use ga_query::indexer::build_index;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct GaRetriever {
    cache_root: PathBuf,
    store: Option<Store>,
}

impl GaRetriever {
    /// `cache_root` is where the lbug graph.db lands. Tests pass a fresh
    /// tempdir; the CLI uses `.graphatlas-bench-cache/<uc>/`.
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            store: None,
        }
    }

    fn store(&self) -> Result<&Store, BenchError> {
        self.store
            .as_ref()
            .ok_or_else(|| BenchError::Query("ga: setup() was not called".into()))
    }
}

impl Retriever for GaRetriever {
    fn name(&self) -> &str {
        "ga"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        // Fresh cache every setup so latencies reflect a cold index — the
        // honest warm/cold story lives in per-query measurement, not here.
        let _ = std::fs::remove_dir_all(&self.cache_root);
        let store = Store::open_with_root(&self.cache_root, fixture_dir)
            .map_err(|e| BenchError::Query(e.to_string()))?;
        build_index(&store, fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("build_index: {e}")))?;
        self.store = Some(store);
        Ok(())
    }

    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        let store = self.store()?;
        match uc {
            "callers" => {
                let symbol = symbol_arg(query, "callers")?;
                let file = query.get("file").and_then(|v| v.as_str());
                let resp = ga_query::callers(store, symbol, file)
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                Ok(resp.callers.into_iter().map(|c| c.symbol).collect())
            }
            "callees" => {
                let symbol = symbol_arg(query, "callees")?;
                let file = query.get("file").and_then(|v| v.as_str());
                let resp = ga_query::callees(store, symbol, file)
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                // Drop externals — bench conventions (AS-005) score callees
                // against in-repo-only expected sets.
                Ok(resp
                    .callees
                    .into_iter()
                    .filter(|c| !c.external)
                    .map(|c| c.symbol)
                    .collect())
            }
            "importers" => {
                let file = file_arg(query, "importers")?;
                let resp = ga_query::importers(store, file)
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                Ok(resp.importers.into_iter().map(|e| e.path).collect())
            }
            "symbols" => {
                let pattern = query
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BenchError::GroundTruthMalformed {
                        path: "<query>".into(),
                        reason: "symbols task missing `pattern`".into(),
                    })?;
                let mode = match query.get("match").and_then(|v| v.as_str()) {
                    Some("fuzzy") => ga_query::SymbolsMatch::Fuzzy,
                    _ => ga_query::SymbolsMatch::Exact,
                };
                let resp = ga_query::symbols(store, pattern, mode)
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                Ok(resp.symbols.into_iter().map(|s| s.name).collect())
            }
            "file_summary" => {
                let path = query.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    BenchError::GroundTruthMalformed {
                        path: "<query>".into(),
                        reason: "file_summary task missing `path`".into(),
                    }
                })?;
                let resp = ga_query::file_summary(store, path)
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                Ok(resp.symbols.into_iter().map(|s| s.name).collect())
            }
            "impact" => {
                let req = impact_request_from_query(query);
                let resp =
                    ga_query::impact(store, &req).map_err(|e| BenchError::Query(e.to_string()))?;
                // Legacy set-based F1 scoring — drop multi-dim metadata.
                // For composite 4-dim scoring the runner uses query_impact().
                Ok(resp.impacted_files.into_iter().map(|f| f.path).collect())
            }
            other => Err(BenchError::UnknownUc(other.to_string())),
        }
    }

    fn teardown(&mut self) {
        self.store = None;
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        let store = match self.store() {
            Ok(s) => s,
            Err(e) => return Some(Err(e)),
        };
        let req = impact_request_from_query(query);
        let resp = match ga_query::impact(store, &req) {
            Ok(r) => r,
            Err(e) => return Some(Err(BenchError::Query(e.to_string()))),
        };
        Some(Ok(ImpactActual {
            files: resp.impacted_files.into_iter().map(|f| f.path).collect(),
            tests: resp.affected_tests.into_iter().map(|t| t.path).collect(),
            routes: resp
                .affected_routes
                .into_iter()
                .map(|r| format!("{} {}", r.method, r.path))
                .collect(),
            transitive_completeness: resp.meta.transitive_completeness,
            max_depth: resp.meta.max_depth,
        }))
    }
}

fn impact_request_from_query(query: &Value) -> ga_query::ImpactRequest {
    ga_query::ImpactRequest {
        symbol: query
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        file: query
            .get("file")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        changed_files: query
            .get("changed_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            }),
        diff: query
            .get("diff")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        max_depth: query
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        // EXP-M2-02 — M2 composite scores only read impacted_files +
        // affected_tests. Skip the 4 non-composite subcomponents to cut
        // 400-800ms/call from bench latency without affecting quality.
        include_break_points: Some(false),
        include_routes: Some(false),
        include_configs: Some(false),
        include_risk: Some(false),
        // EXP-M2-11 — keep default (false) in gate bench: feature is opt-in
        // only. Lifts blast_radius +38% but blows p95 4×; composite drops
        // −0.012 due to strict-precision impact of new pool. Callers that
        // want LLM-agent utility (blast_radius) explicitly set Some(true).
        include_co_change_importers: Some(false),
    }
}

fn symbol_arg<'a>(query: &'a Value, uc: &str) -> Result<&'a str, BenchError> {
    query
        .get("symbol")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BenchError::GroundTruthMalformed {
            path: "<query>".into(),
            reason: format!("{uc} task missing `symbol`"),
        })
}

fn file_arg<'a>(query: &'a Value, uc: &str) -> Result<&'a str, BenchError> {
    query
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BenchError::GroundTruthMalformed {
            path: "<query>".into(),
            reason: format!("{uc} task missing `file`"),
        })
}
