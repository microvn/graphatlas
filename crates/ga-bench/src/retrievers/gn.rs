//! GitNexus retriever — MCP client over `gitnexus mcp`.
//!
//! Per-UC mapping (validated 2026-05-23 via 5×5 audit):
//!   - callers   → `context {name, repo}` — read incoming.* (calls/imports/has_method/extends)
//!   - callees   → `context {name, repo}` — read outgoing.*
//!   - impact    → `impact {target, direction:"both", repo}` → impactedFiles
//!   - symbols   → `query {query, repo}` — BM25 + vector hybrid; returns process flows
//!   - file_summary / importers → no native tool → returns Vec::new()
//!
//! Quirks (verified empirically):
//!   - `query` field is named `query`, NOT `search_query`
//!   - `impact` requires `direction` param (use `"both"`)
//!   - Multi-repo state: when ≥ 2 repos indexed, MUST pass `repo: <alias>`. Auto-detect via
//!     `gitnexus list` stdout (TSV: alias / path / status).
//!   - Response has JSON body THEN trailing markdown ("**Next:**..."). Use balanced-brace
//!     extraction (`extract_balanced_json`) — `serde_json::from_str` on raw text fails.
//!   - Status `"ambiguous"` returned for multi-def symbols; candidates list useful as
//!     fallback in callers UC (the names are caller candidates user must disambiguate).
//!
//! Pre-index requirement: user runs `gitnexus analyze <fixture>` once per fixture before
//! bench time. Retriever does NOT write to GN's graph.
//!
//! Availability: `setup()` probes `gitnexus --version`. Missing → retriever disabled,
//! all queries return empty (leaderboard renders entry with `pass_rate = 0`).

use crate::mcp::{McpChild, McpError};
use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use ga_query::common::is_test_path;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const INIT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct GnRetriever {
    available: bool,
    fixture_dir: Option<PathBuf>,
    /// Auto-detected GN repo alias (e.g. "rotor", "gin", "tokio"). Required when
    /// multiple repos indexed. None on single-repo setups or when `gitnexus list`
    /// produces no matching path.
    repo_alias: Option<String>,
    child: Option<McpChild>,
}

impl Default for GnRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl GnRetriever {
    pub fn new() -> Self {
        Self {
            available: false,
            fixture_dir: None,
            repo_alias: None,
            child: None,
        }
    }

    /// Walk `gitnexus list` output and find the alias mapped to `fixture_dir`.
    ///
    /// Output format (verified 2026-05-23 with gitnexus 1.6.5):
    /// ```text
    ///   <alias>[  (<disambig-path>)]
    ///       Path:    <abspath>
    ///       Indexed: <date>
    ///       Commit:  <sha>
    ///       Stats:   ...
    /// ```
    /// Alias lines have 2-space indent; field lines have 4-space indent. We
    /// walk stanzas, remember the current alias when we see a 2-indent line,
    /// then check `Path:` lines against `fixture_dir`.
    fn detect_repo_alias(fixture_dir: &Path) -> Option<String> {
        let out = Command::new("gitnexus").arg("list").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let canon = fixture_dir.canonicalize().ok()?;
        let mut current_alias: Option<String> = None;
        for line in text.lines() {
            // Skip blanks and header ("  Indexed Repositories (N)").
            if line.trim().is_empty() {
                continue;
            }
            // 2-space indent (not 4) and not the field rows = new alias header.
            let is_alias_row = line.starts_with("  ") && !line.starts_with("    ");
            if is_alias_row {
                let first = line.split_whitespace().next().unwrap_or("");
                // Skip the "Indexed Repositories" banner line.
                if first.starts_with(char::is_alphabetic) && !first.eq_ignore_ascii_case("Indexed")
                {
                    current_alias = Some(first.to_string());
                }
                continue;
            }
            // 4-space indented field row — check for `Path:    <abspath>`.
            if let Some(rest) = line.trim_start().strip_prefix("Path:") {
                let path_str = rest.trim();
                if let Ok(p) = Path::new(path_str).canonicalize() {
                    if p == canon {
                        return current_alias.clone();
                    }
                }
            }
        }
        None
    }
}

impl Retriever for GnRetriever {
    fn name(&self) -> &str {
        "gitnexus"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_dir = Some(fixture_dir.to_path_buf());

        let present = Command::new("which")
            .arg("gitnexus")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !present {
            self.available = false;
            return Ok(());
        }

        self.repo_alias = Self::detect_repo_alias(fixture_dir);

        match McpChild::spawn(&["gitnexus", "mcp"], INIT_TIMEOUT) {
            Ok(child) => {
                self.child = Some(child);
                self.available = true;
                Ok(())
            }
            Err(e) => {
                eprintln!("gn: spawn failed: {e} — retriever disabled");
                self.available = false;
                Ok(())
            }
        }
    }

    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        if !self.available {
            return Ok(Vec::new());
        }
        let Some(repo) = self.repo_alias.clone() else {
            // Multi-repo setup without alias = can't disambiguate → empty
            return Ok(Vec::new());
        };
        let Some(child) = self.child.as_mut() else {
            return Ok(Vec::new());
        };
        let Some((method, args)) = build_gn_request(uc, query, &repo) else {
            return Ok(Vec::new());
        };
        match child.tools_call(method, args, CALL_TIMEOUT) {
            Ok(resp) => Ok(extract_names_from_response(&resp, uc)),
            Err(McpError::Timeout(_)) => {
                eprintln!("gn: timeout on uc={uc}");
                Ok(Vec::new())
            }
            Err(e) => {
                eprintln!("gn: query error ({uc}): {e}");
                Ok(Vec::new())
            }
        }
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        if !self.available {
            return Some(Ok(ImpactActual::default()));
        }
        let target = query.get("symbol").and_then(|v| v.as_str())?;
        let repo = self.repo_alias.clone()?;
        let child = self.child.as_mut()?;
        let args = json!({"target": target, "direction": "both", "repo": repo});
        let resp = match child.tools_call("impact", args, CALL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => return Some(Ok(ImpactActual::default())),
        };
        let all = extract_impact_files(&resp);
        // Mirror CmRetriever fairness: partition by is_test_path so tests dim isn't
        // unfairly zeroed when GN actually surfaces test-file refs.
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

// ───── helpers (pub for unit tests) ─────

pub fn build_gn_request(uc: &str, query: &Value, repo: &str) -> Option<(&'static str, Value)> {
    match uc {
        "callers" | "callees" => {
            let symbol = query.get("symbol").and_then(|v| v.as_str())?;
            Some(("context", json!({"name": symbol, "repo": repo})))
        }
        "symbols" => {
            let pattern = query.get("pattern").and_then(|v| v.as_str())?;
            Some(("query", json!({"query": pattern, "repo": repo})))
        }
        _ => None,
    }
}

/// Extract names from GN MCP response. Handles:
/// - JSON body followed by trailing markdown (balanced-brace extraction)
/// - `status: "found"` → walk incoming.* (callers) or outgoing.* (callees)
/// - `status: "ambiguous"` → return candidates as fallback
/// - `query` tool returns `processes[].summary` strings
pub fn extract_names_from_response(response: &Value, uc: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(content) = response.get("content").and_then(|v| v.as_array()) else {
        return out;
    };
    for block in content {
        let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        let body = match extract_balanced_json(text) {
            Some(b) => b,
            None => continue,
        };
        let Ok(d) = serde_json::from_str::<Value>(&body) else {
            continue;
        };

        match uc {
            "callers" => walk_relation_names(&d, "incoming", &mut out),
            "callees" => walk_relation_names(&d, "outgoing", &mut out),
            "symbols" => walk_query_processes(&d, &mut out),
            _ => {}
        }

        // Ambiguous-symbol fallback: surface candidate names regardless of UC
        if d.get("status").and_then(|s| s.as_str()) == Some("ambiguous") {
            if let Some(cands) = d.get("candidates").and_then(|v| v.as_array()) {
                for c in cands {
                    if let Some(name) = c.get("name").and_then(|v| v.as_str()) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn walk_relation_names(d: &Value, key: &str, out: &mut Vec<String>) {
    let Some(rel) = d.get(key).and_then(|v| v.as_object()) else {
        return;
    };
    for (_edge_kind, items) in rel {
        let Some(arr) = items.as_array() else {
            continue;
        };
        for it in arr {
            if let Some(name) = it.get("name").and_then(|v| v.as_str()) {
                out.push(name.to_string());
            }
        }
    }
}

fn walk_query_processes(d: &Value, out: &mut Vec<String>) {
    if let Some(processes) = d.get("processes").and_then(|v| v.as_array()) {
        for p in processes {
            if let Some(summary) = p.get("summary").and_then(|v| v.as_str()) {
                out.push(summary.to_string());
            }
        }
    }
    // Alternative shape: `symbols[]` direct (some GN versions)
    if let Some(syms) = d.get("symbols").and_then(|v| v.as_array()) {
        for s in syms {
            if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
                out.push(name.to_string());
            }
        }
    }
}

fn extract_impact_files(response: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(content) = response.get("content").and_then(|v| v.as_array()) else {
        return out;
    };
    for block in content {
        let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
            continue;
        };
        let body = match extract_balanced_json(text) {
            Some(b) => b,
            None => continue,
        };
        let Ok(d) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        // Walk `impacted[]` / `impactedFiles[]` arrays
        for key in ["impacted", "impactedFiles"] {
            if let Some(arr) = d.get(key).and_then(|v| v.as_array()) {
                for it in arr {
                    let f = it
                        .get("filePath")
                        .or_else(|| it.get("file_path"))
                        .or_else(|| it.get("file"))
                        .and_then(|v| v.as_str());
                    if let Some(path) = f {
                        out.push(path.to_string());
                    } else if let Some(s) = it.as_str() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Find first balanced `{...}` object in text. Used to strip GitNexus trailing
/// markdown. Returns None if no `{` or unbalanced.
pub fn extract_balanced_json(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0i32;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_balanced_json_strips_trailing_markdown() {
        let raw = r#"{"status":"found","name":"x"}

**Next:** call context() for deeper view."#;
        let body = extract_balanced_json(raw).unwrap();
        assert_eq!(body, r#"{"status":"found","name":"x"}"#);
    }

    #[test]
    fn extract_balanced_json_handles_nested_objects() {
        let raw = r#"{"a":{"b":{"c":1}},"d":2}\n**trailer**"#;
        let body = extract_balanced_json(raw).unwrap();
        assert_eq!(body, r#"{"a":{"b":{"c":1}},"d":2}"#);
    }

    #[test]
    fn build_gn_callers_uses_context() {
        let q = json!({"symbol": "encrypt"});
        let (method, args) = build_gn_request("callers", &q, "rotor").unwrap();
        assert_eq!(method, "context");
        assert_eq!(args["name"], "encrypt");
        assert_eq!(args["repo"], "rotor");
    }

    #[test]
    fn build_gn_symbols_uses_query() {
        let q = json!({"pattern": "Endpoint"});
        let (method, args) = build_gn_request("symbols", &q, "rotor").unwrap();
        assert_eq!(method, "query");
        assert_eq!(args["query"], "Endpoint");
    }

    #[test]
    fn extract_names_callers_walks_incoming_calls() {
        let resp = json!({
            "content": [{
                "text": r#"{
                    "status": "found",
                    "symbol": {"name": "encrypt"},
                    "incoming": {
                        "calls": [
                            {"name": "iframeKeypair", "filePath": "a.ts"},
                            {"name": "adminAccounts", "filePath": "b.ts"}
                        ]
                    }
                }
                **Next:** ..."#
            }]
        });
        let names = extract_names_from_response(&resp, "callers");
        assert_eq!(names, vec!["adminAccounts", "iframeKeypair"]);
    }

    #[test]
    fn extract_names_ambiguous_returns_candidates() {
        let resp = json!({
            "content": [{
                "text": r#"{
                    "status": "ambiguous",
                    "candidates": [
                        {"name": "Default", "filePath": "gin.go"},
                        {"name": "Default", "filePath": "binding/binding.go"}
                    ]
                }"#
            }]
        });
        let names = extract_names_from_response(&resp, "callers");
        // Dedup happens after sort; both candidates have name "Default" → 1 unique entry
        assert_eq!(names, vec!["Default"]);
    }
}
