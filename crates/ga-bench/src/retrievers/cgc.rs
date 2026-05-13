//! CodeGraphContext retriever — MCP client over `cgc mcp start`.
//!
//! Per-UC mapping (adapted from `src/adapters/codegraphcontext.ts`):
//!   - callers    → `analyze_code_relationships {query_type:"find_callers", target}`
//!   - callees    → `analyze_code_relationships {query_type:"find_callees", target}`
//!   - importers  → `analyze_code_relationships {query_type:"find_importers", target:module_stem}`
//!   - symbols    → `find_code {query: pattern}`
//!   - file_summary → no native support → query returns Vec::new()
//!
//! Availability: `setup()` probes `cgc --version`. If cgc missing, the
//! retriever stores `available=false` and every `query()` returns empty —
//! leaderboard still renders the entry, pass_rate = 0.
//!
//! Response parsing is flexible by design: different cgc versions return
//! slightly different field names. The extractor walks the content.text JSON
//! looking for the per-UC name fields (`caller_name` / `callee_name` /
//! `file_path` / ...). Empty extraction → task scored as 0, leaderboard shows
//! real degradation rather than hiding it.

use crate::mcp::{McpChild, McpError};
use crate::retriever::Retriever;
use crate::BenchError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const INIT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CgcRetriever {
    available: bool,
    fixture_dir: Option<PathBuf>,
    child: Option<McpChild>,
}

impl Default for CgcRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl CgcRetriever {
    pub fn new() -> Self {
        Self {
            available: false,
            fixture_dir: None,
            child: None,
        }
    }
}

impl Retriever for CgcRetriever {
    fn name(&self) -> &str {
        "codegraphcontext"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_dir = Some(fixture_dir.to_path_buf());

        // Probe availability — `cgc --version` is quick + doesn't mutate.
        // If absent, we disable gracefully.
        let probe = Command::new("cgc").arg("--version").output();
        let present = probe.as_ref().map(|o| o.status.success()).unwrap_or(false);
        if !present {
            self.available = false;
            return Ok(());
        }

        // IMPORTANT: the retriever is QUERY-ONLY. It does NOT write to CGC's
        // graph. Pre-indexing is a separate prerequisite the user runs once:
        //   cgc index /path/to/fixture
        // before benching. Bench-C7 graceful-disable still applies when the
        // fixture isn't indexed — find_callers will simply return 0 results,
        // leaderboard shows pass_rate=0 with an honest diagnostic, not
        // silently auto-index behind the user's back.

        // Spawn the persistent MCP server process.
        match McpChild::spawn(&["cgc", "mcp", "start"], INIT_TIMEOUT) {
            Ok(child) => {
                self.child = Some(child);
                self.available = true;
                Ok(())
            }
            Err(e) => {
                eprintln!("cgc: spawn failed: {e} — retriever disabled for this run");
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

        // Importers UC has a convention mismatch: CGC's `find_importers`
        // expects module NAMES (import strings like `@nestjs/core`,
        // `./rel/path`), not file paths. We resolve candidate module names
        // for the file (package.json lookup + basename + stem) and try each
        // via `find_importers`, accumulating distinct importer files.
        if uc == "importers" {
            if let Some(file) = query.get("file").and_then(|v| v.as_str()) {
                // Clone fixture_dir to drop the immutable borrow before
                // re-borrowing self mutably inside the helper.
                let fixture_owned = fixture_dir.clone();
                return Ok(self.importers_file_via_modules(&fixture_owned, file));
            }
        }

        // Impact UC approximation: CGC has no native impact tool, but we can
        // derive a reasonable file-set by UNIONING find_callers + find_callees
        // on the seed symbol. Not as sophisticated as GA's multi-signal
        // fusion, but honest "CGC doing impact via the tools it has."
        if uc == "impact" {
            return Ok(self.impact_via_callers_callees(query, &fixture_str));
        }

        let Some((method, mut args)) = build_cgc_request(uc, query, &fixture_str) else {
            return Ok(Vec::new());
        };
        if args.get("repo_path").is_none() {
            args["repo_path"] = Value::String(fixture_str);
        }
        let Some(child) = self.child.as_mut() else {
            return Ok(Vec::new());
        };
        match child.tools_call(method, args, CALL_TIMEOUT) {
            Ok(resp) => Ok(extract_names_from_response(&resp, uc)),
            Err(McpError::Timeout(_)) => {
                eprintln!("cgc: timeout on uc={uc}");
                Ok(Vec::new())
            }
            Err(e) => {
                eprintln!("cgc: query error ({uc}): {e}");
                Ok(Vec::new())
            }
        }
    }

    fn teardown(&mut self) {
        // Drop the child — its Drop impl runs the graceful-shutdown dance.
        self.child = None;
    }
}

impl CgcRetriever {
    /// Impact UC approximation via CGC's native tools. CGC doesn't have a
    /// native impact analyzer — we proxy it by querying find_callers +
    /// find_callees on the seed symbol and unioning the returned file paths.
    /// Honest "CGC does impact with the structural tools it has", not a
    /// custom CGC-specific optimization.
    fn impact_via_callers_callees(&mut self, query: &Value, repo_path: &str) -> Vec<String> {
        let Some(symbol) = query.get("symbol").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let Some(child) = self.child.as_mut() else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for query_type in &["find_callers", "find_callees"] {
            let args = json!({
                "query_type": query_type,
                "target": symbol,
                "repo_path": repo_path,
            });
            match child.tools_call("analyze_code_relationships", args, CALL_TIMEOUT) {
                Ok(resp) => {
                    for path in extract_file_paths_from_response(&resp) {
                        let rel = path
                            .strip_prefix(&format!("{}/", repo_path))
                            .unwrap_or(&path)
                            .to_string();
                        if seen.insert(rel.clone()) {
                            out.push(rel);
                        }
                    }
                }
                Err(_) => continue,
            }
        }
        // No seed-file fallback — seed is GT-derived, not a tool result.
        // An empty response reflects the tool's real capability on this task.
        out
    }

    /// File-path importers lookup that bridges CGC's module-first model:
    ///   1. Enumerate plausible module names for the file (package.json
    ///      workspace scoped name + path stems + basename).
    ///   2. `find_importers(target=<module>)` for each candidate.
    ///   3. Accumulate distinct importer file paths (repo-relative).
    ///
    /// Returns empty if none of the candidates match — honest miss, not
    /// silent failure.
    fn importers_file_via_modules(&mut self, fixture_dir: &Path, file: &str) -> Vec<String> {
        let candidates = module_candidates_for_file(fixture_dir, file);
        let Some(child) = self.child.as_mut() else {
            return Vec::new();
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let fixture_prefix = format!("{}/", fixture_dir.display());
        for cand in candidates {
            let args = json!({
                "query_type": "find_importers",
                "target": cand,
                "repo_path": fixture_dir.display().to_string(),
            });
            match child.tools_call("analyze_code_relationships", args, CALL_TIMEOUT) {
                Ok(resp) => {
                    for path in extract_file_paths_from_response(&resp) {
                        let rel = path
                            .strip_prefix(&fixture_prefix)
                            .unwrap_or(&path)
                            .to_string();
                        if seen.insert(rel.clone()) {
                            out.push(rel);
                        }
                    }
                }
                Err(_) => continue,
            }
        }
        out
    }
}

/// Walk up from `fixture_dir/<rel_file>` looking for a `package.json`. If
/// found and it has a `name` field, produce `{name}/<rel from pkg root>`.
/// Always includes basename + file-stem as additional candidates.
fn module_candidates_for_file(fixture_dir: &Path, rel_file: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let file_abs = fixture_dir.join(rel_file);

    // package.json climb
    let mut cursor = file_abs.parent();
    while let Some(dir) = cursor {
        if dir == fixture_dir.parent().unwrap_or(Path::new("/")) {
            break;
        }
        let pkg_path = dir.join("package.json");
        if pkg_path.is_file() {
            if let Ok(raw) = std::fs::read(&pkg_path) {
                if let Ok(v) = serde_json::from_slice::<Value>(&raw) {
                    if let Some(pkg_name) = v.get("name").and_then(|n| n.as_str()) {
                        if let Ok(rel_from_root) = file_abs.strip_prefix(dir) {
                            let rel_s = rel_from_root.to_string_lossy().into_owned();
                            // index.ts / src/index.ts → bare package name
                            let trimmed = rel_s
                                .trim_end_matches(".ts")
                                .trim_end_matches(".tsx")
                                .trim_end_matches(".js")
                                .trim_end_matches(".jsx")
                                .trim_end_matches("/index")
                                .trim_start_matches("src/")
                                .trim_end_matches("/index");
                            if trimmed.is_empty() || trimmed == "index" {
                                out.push(pkg_name.to_string());
                            } else {
                                out.push(format!("{}/{}", pkg_name, trimmed));
                                out.push(pkg_name.to_string()); // also try bare
                            }
                        }
                    }
                }
            }
            break;
        }
        cursor = dir.parent();
    }

    // Fallback candidates — basename + stem — useful for Python modules +
    // cases where no package.json exists.
    if let Some(stem) = Path::new(rel_file).file_stem().and_then(|s| s.to_str()) {
        if stem != "index" && stem.len() >= 3 {
            out.push(stem.to_string());
        }
    }

    // Dedupe preserving order
    let mut seen = std::collections::HashSet::new();
    out.retain(|x| seen.insert(x.clone()));
    out
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
            &[
                "file_path",
                "filePath",
                "path",
                "caller_file_path",
                "target_file_path",
            ],
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

/// Build the CGC MCP request (method name + arguments) for a given UC + task
/// query. Returns `None` when CGC has no plausible handler (e.g. file_summary).
///
/// Pure function — all state lives in the returned tuple. Testable without
/// spawning a child.
pub fn build_cgc_request(
    uc: &str,
    query: &Value,
    repo_path: &str,
) -> Option<(&'static str, Value)> {
    match uc {
        "callers" => {
            let symbol = query.get("symbol").and_then(|v| v.as_str())?;
            Some((
                "analyze_code_relationships",
                json!({
                    "query_type": "find_callers",
                    "target": symbol,
                    "repo_path": repo_path,
                }),
            ))
        }
        "callees" => {
            let symbol = query.get("symbol").and_then(|v| v.as_str())?;
            Some((
                "analyze_code_relationships",
                json!({
                    "query_type": "find_callees",
                    "target": symbol,
                    "repo_path": repo_path,
                }),
            ))
        }
        "importers" => {
            // TS adapter: strip dirs + extension → module stem. Mirror that
            // so bench GT authored in this project's layout doesn't have to
            // know CGC's internal target shape.
            let file = query.get("file").and_then(|v| v.as_str())?;
            let stem = std::path::Path::new(file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(file);
            Some((
                "analyze_code_relationships",
                json!({
                    "query_type": "find_importers",
                    "target": stem,
                    "repo_path": repo_path,
                }),
            ))
        }
        "symbols" => {
            let pattern = query.get("pattern").and_then(|v| v.as_str())?;
            Some((
                "find_code",
                json!({
                    "query": pattern,
                    "repo_path": repo_path,
                }),
            ))
        }
        _ => None, // file_summary and any unknown UC
    }
}

/// Walk a CGC MCP response (content blocks) and extract names relevant to
/// `uc`. Returns an empty Vec when the response is degraded (raw text only,
/// empty content, unexpected shape) — scorer treats as 0 without panicking.
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
            // Raw text fallback — skip; the bench is honest about degradation.
            continue;
        };
        collect_named_fields(&parsed, fields, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn name_fields_for_uc(uc: &str) -> &'static [&'static str] {
    // Field names discovered from probing `cgc mcp start` response shapes
    // against axum fixture (2026-04-21). CGC uses `caller_function` /
    // `callee_function`, not `caller_name`.
    match uc {
        "callers" => &["caller_function", "caller_name", "name", "symbol"],
        "callees" => &["callee_function", "callee_name", "name", "symbol"],
        "importers" => &[
            "file_path",
            "filePath",
            "importer_file",
            "source_file",
            "path",
        ],
        "symbols" => &["symbol_name", "name"],
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
