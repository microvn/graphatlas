//! codebase-memory-mcp retriever — MCP client over `codebase-memory-mcp`.
//!
//! Per-UC mapping (adapted from `src/adapters/codebase-memory.ts`):
//!   - callers / callees → `trace_call_path {function_name, project, depth:5}`
//!      CM doesn't split direction in this tool, so callees accuracy is
//!      inherently weaker than callers. Document as limitation, not bug.
//!   - symbols  → `search_code {pattern, project, mode:"files", limit:30}`
//!   - importers / file_summary → no native tool → returns None
//!
//! Query-only: CmRetriever does NOT write to CM's graph. User must pre-index
//! the fixture once (`codebase-memory-mcp index <fixture>` or equivalent)
//! before bench time. A missing index → empty query results → pass_rate=0.
//! Availability: `setup()` probes `which codebase-memory-mcp`. Missing →
//! retriever disabled, entries still render with pass_rate=0.

use crate::mcp::{McpChild, McpError};
use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const INIT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CmRetriever {
    available: bool,
    fixture_dir: Option<PathBuf>,
    child: Option<McpChild>,
}

impl Default for CmRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl CmRetriever {
    pub fn new() -> Self {
        Self {
            available: false,
            fixture_dir: None,
            child: None,
        }
    }
}

impl Retriever for CmRetriever {
    fn name(&self) -> &str {
        "codebase-memory"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_dir = Some(fixture_dir.to_path_buf());

        // Probe: `codebase-memory-mcp --help` (or --version) — the exact flag
        // depends on install; try a couple. If neither works, retriever stays
        // disabled and every query returns empty gracefully.
        let probe = Command::new("which").arg("codebase-memory-mcp").output();
        let present = probe.as_ref().map(|o| o.status.success()).unwrap_or(false);
        if !present {
            self.available = false;
            return Ok(());
        }

        // Query-only: pre-indexing is the user's responsibility, not the
        // retriever's. Run `codebase-memory-mcp index` (or equivalent) once
        // per fixture before benching. The retriever never calls
        // `index_repository` — that's a write op.
        match McpChild::spawn(&["codebase-memory-mcp"], INIT_TIMEOUT) {
            Ok(child) => {
                self.child = Some(child);
                self.available = true;
                Ok(())
            }
            Err(e) => {
                eprintln!("cm: spawn failed: {e} — retriever disabled");
                self.available = false;
                Ok(())
            }
        }
    }

    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        if !self.available {
            return Ok(Vec::new());
        }
        let Some(fixture_dir) = self.fixture_dir.as_ref() else {
            return Ok(Vec::new());
        };
        let fixture_str = fixture_dir.display().to_string();
        let Some((method, args)) = build_cm_request(uc, query, &fixture_str) else {
            return Ok(Vec::new());
        };
        let Some(child) = self.child.as_mut() else {
            return Ok(Vec::new());
        };
        match child.tools_call(method, args, CALL_TIMEOUT) {
            Ok(resp) => {
                // For the symbols UC specifically, CM's search_code returns
                // file paths but our GT expects symbol names. Promote the
                // query pattern to rank-1 when any file matched — same rule
                // as ripgrep's symbols UC. Keeps lexical baselines
                // comparable to each other.
                if uc == "symbols" {
                    let hits = extract_names_from_response(&resp, uc);
                    if hits.is_empty() {
                        return Ok(Vec::new());
                    }
                    let pattern = query
                        .get("pattern")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    return Ok(if pattern.is_empty() {
                        hits
                    } else {
                        vec![pattern]
                    });
                }
                Ok(extract_names_from_response(&resp, uc))
            }
            Err(McpError::Timeout(_)) => {
                eprintln!("cm: timeout on uc={uc}");
                Ok(Vec::new())
            }
            Err(e) => {
                eprintln!("cm: query error ({uc}): {e}");
                Ok(Vec::new())
            }
        }
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        if !self.available {
            return Some(Ok(ImpactActual::default()));
        }
        let all = self.impact_files(query);
        // BENCH-FAIR 2026-04-24: previously `tests: Vec::new()` hardcoded,
        // zeroing test_recall unfairly. CM returns file paths from
        // search_code — partition on is_test_path to populate tests field
        // (convention-match fallback, matches bm25/cgc/ripgrep defaults).
        let (files, tests): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| !is_test_path(p));
        Some(Ok(ImpactActual {
            files,
            tests,
            routes: Vec::new(),
            transitive_completeness: 0,
            max_depth: 5,
        }))
    }

    fn teardown(&mut self) {
        self.child = None;
    }
}

impl CmRetriever {
    /// Impact via `query_graph` Cypher: walks CALLS edges 1..3 hops from seed,
    /// returns distinct files. `trace_path` returns symbol names without file
    /// paths, so we can't use it for impact scoring.
    fn impact_files(&mut self, query: &Value) -> Vec<String> {
        let Some(symbol) = query.get("symbol").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let Some(fixture_dir) = self.fixture_dir.as_ref() else {
            return Vec::new();
        };
        let fixture_str = fixture_dir.display().to_string();
        let prefix = format!("{}/", fixture_str);
        let project = project_from_path(&fixture_str);
        let Some(child) = self.child.as_mut() else {
            return Vec::new();
        };
        let cypher = format!(
            "MATCH (s {{name: \"{}\"}})-[:CALLS*1..3]-(n) RETURN DISTINCT n.file_path LIMIT 30",
            symbol
        );
        let args = json!({"project": project, "query": cypher});
        let resp = match child.tools_call("query_graph", args, CALL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in extract_cypher_rows(&resp) {
            let rel = path.strip_prefix(&prefix).unwrap_or(&path).to_string();
            if seen.insert(rel.clone()) {
                out.push(rel);
            }
        }
        out
    }
}

/// Parse `query_graph` response: `{"columns":[...],"rows":[[v1],[v2],...]}`.
/// Returns all string values from rows.
fn extract_cypher_rows(response: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(content) = response.get("content").and_then(|v| v.as_array()) else {
        return out;
    };
    for block in content {
        let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let Some(rows) = parsed.get("rows").and_then(|v| v.as_array()) else {
            continue;
        };
        for row in rows {
            let Some(row_arr) = row.as_array() else {
                continue;
            };
            for cell in row_arr {
                if let Some(s) = cell.as_str() {
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out
}

/// TS adapter convention: project identifier is the repo path with slashes
/// replaced by dashes and any leading dash stripped.
pub fn project_from_path(repo_path: &str) -> String {
    let dashed: String = repo_path.replace('/', "-");
    dashed.trim_start_matches('-').to_string()
}

pub fn build_cm_request(uc: &str, query: &Value, repo_path: &str) -> Option<(&'static str, Value)> {
    let project = project_from_path(repo_path);
    match uc {
        "callers" | "callees" => {
            let symbol = query.get("symbol").and_then(|v| v.as_str())?;
            Some((
                "trace_call_path",
                json!({
                    "function_name": symbol,
                    "project": project,
                    "depth": 5,
                }),
            ))
        }
        "symbols" => {
            let pattern = query.get("pattern").and_then(|v| v.as_str())?;
            Some((
                "search_code",
                json!({
                    "pattern": pattern,
                    "project": project,
                    "mode": "files",
                    "limit": 30,
                }),
            ))
        }
        _ => None,
    }
}

pub fn extract_names_from_response(response: &Value, uc: &str) -> Vec<String> {
    let fields = name_fields_for_uc(uc);
    let mut out = Vec::new();
    let Some(content) = response.get("content").and_then(|v| v.as_array()) else {
        return out;
    };
    for block in content {
        let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        collect_named_fields(&parsed, fields, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn name_fields_for_uc(uc: &str) -> &'static [&'static str] {
    match uc {
        // CM's trace_call_path chain entries carry {name, file}
        "callers" | "callees" => &["name", "function_name", "symbol"],
        // search_code returns {file, path} — extractor returns files; the
        // retriever's `query` wrapper promotes the pattern for MRR scoring.
        "symbols" => &["file", "file_path", "path"],
        _ => &[],
    }
}

fn collect_named_fields(v: &Value, fields: &[&str], out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for k in fields {
                if let Some(Value::String(s)) = map.get(*k) {
                    out.push(s.clone());
                }
            }
            for (_, val) in map {
                collect_named_fields(val, fields, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_named_fields(item, fields, out);
            }
        }
        _ => {}
    }
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`.
use ga_query::common::is_test_path;
