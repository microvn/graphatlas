//! code-review-graph retriever — MCP client over `code-review-graph serve`.
//!
//! Per-UC mapping:
//!   - impact      → `get_impact_radius_tool {changed_files:[seed_file], repo_root}`
//!                   (direct mapping: CRG's purpose-built blast-radius tool)
//!   - callers/callees/importers/symbols/file_summary → no native atomic tools;
//!                   query returns Vec::new() → leaderboard pass_rate=0, honest.
//!
//! Availability: `setup()` probes `code-review-graph --version`. Missing →
//! `available=false`, every query returns empty.
//!
//! Pre-build: first `setup()` call per fixture runs `build_or_update_graph_tool`
//! via MCP to populate CRG's sqlite graph cache. Subsequent queries use the
//! persisted graph.
//!
//! Response parsing: CRG returns file paths in various fields depending on the
//! tool. We walk the JSON looking for `file`, `path`, `file_path`.

use crate::mcp::McpChild;
use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const INIT_TIMEOUT: Duration = Duration::from_secs(15);
const BUILD_TIMEOUT: Duration = Duration::from_secs(120);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CrgRetriever {
    available: bool,
    fixture_dir: Option<PathBuf>,
    child: Option<McpChild>,
}

impl Default for CrgRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl CrgRetriever {
    pub fn new() -> Self {
        Self {
            available: false,
            fixture_dir: None,
            child: None,
        }
    }
}

impl Retriever for CrgRetriever {
    fn name(&self) -> &str {
        "code-review-graph"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_dir = Some(fixture_dir.to_path_buf());

        let probe = Command::new("code-review-graph").arg("--version").output();
        let present = probe.as_ref().map(|o| o.status.success()).unwrap_or(false);
        if !present {
            self.available = false;
            return Ok(());
        }

        match McpChild::spawn(&["code-review-graph", "serve"], INIT_TIMEOUT) {
            Ok(mut child) => {
                // Pre-build the graph for this fixture so subsequent
                // get_impact_radius_tool calls have data. BENCH-FAIR
                // 2026-05-04: pass `full_rebuild: true` because GA's
                // runner checks out a different `base_commit` per task
                // (m2_runner.rs:144); without full rebuild, CRG's
                // incremental update path leaks stale node entries from
                // prior tasks' commit states into the next task's graph.
                let build_args = json!({
                    "repo_root": fixture_dir.display().to_string(),
                    "full_rebuild": true,
                });
                if let Err(e) =
                    child.tools_call("build_or_update_graph_tool", build_args, BUILD_TIMEOUT)
                {
                    eprintln!(
                        "code-review-graph: build_or_update_graph_tool failed on {}: {e} \
                         — queries may return empty",
                        fixture_dir.display()
                    );
                }
                self.child = Some(child);
                self.available = true;
                Ok(())
            }
            Err(e) => {
                eprintln!("code-review-graph: spawn failed: {e} — retriever disabled");
                self.available = false;
                Ok(())
            }
        }
    }

    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        if !self.available {
            return Ok(Vec::new());
        }
        if uc != "impact" {
            return Ok(Vec::new());
        }
        Ok(self.impact_files(query))
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        if !self.available {
            return Some(Ok(ImpactActual::default()));
        }
        let all = self.impact_files(query);
        // BENCH-FAIR 2026-04-24: previously `tests: Vec::new()` hardcoded,
        // which zeroed test_recall unfairly. CRG returns file paths via
        // get_impact_radius_tool — partition on is_test_path so test files
        // count toward test_recall (convention-match fallback, matches
        // what bm25/cgc/ripgrep do via the default fallback path).
        let (files, tests): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| !is_test_path(p));
        Some(Ok(ImpactActual {
            files,
            tests,
            routes: Vec::new(),
            transitive_completeness: 0,
            max_depth: 3,
        }))
    }

    fn teardown(&mut self) {
        self.child = None;
    }
}

impl CrgRetriever {
    fn impact_files(&mut self, query: &Value) -> Vec<String> {
        let Some(file) = query.get("file").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let Some(fixture_dir) = self.fixture_dir.as_ref() else {
            return Vec::new();
        };
        let repo_root = fixture_dir.display().to_string();
        let prefix = format!("{}/", repo_root);
        let Some(child) = self.child.as_mut() else {
            return Vec::new();
        };
        // M2 is a per-symbol bench — pass single seed file. An earlier
        // attempt to read `changed_files` from query (and project the GT's
        // `source_files`) was reverted 2026-05-04 after detecting that
        // `source_files == expected_files` by GT construction → recall
        // tautology. See methodology.md §Fairness audit log.
        // `detail_level: "standard"` is CRG's canonical full-output mode
        // (tools/query.py:51, 99-117); the prior `"full"` string fell
        // through to the same branch coincidentally but was never a
        // documented value.
        let args = json!({
            "changed_files": [file],
            "repo_root": repo_root,
            "max_depth": 3,
            "detail_level": "standard",
        });
        let resp = match child.tools_call("get_impact_radius_tool", args, CALL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for path in extract_file_paths_from_response(&resp) {
            let rel = path.strip_prefix(&prefix).unwrap_or(&path).to_string();
            if seen.insert(rel.clone()) {
                out.push(rel);
            }
        }
        let _ = file; // seed file is GT-derived, NOT a tool result — no fallback
        out
    }
}

fn extract_file_paths_from_response(response: &Value) -> Vec<String> {
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
        collect_fields(
            &parsed,
            &["file_path", "filePath", "path", "file"],
            &mut out,
        );
    }
    out
}

fn collect_fields(v: &Value, fields: &[&str], out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for k in fields {
                if let Some(Value::String(s)) = map.get(*k) {
                    out.push(s.clone());
                }
            }
            for (_, val) in map {
                collect_fields(val, fields, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_fields(item, fields, out);
            }
        }
        _ => {}
    }
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`.
use ga_query::common::is_test_path;
