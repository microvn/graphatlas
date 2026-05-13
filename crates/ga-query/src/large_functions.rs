//! `ga_large_functions` — symbols whose line span exceeds a threshold.
//!
//! Mirrors `code-review-graph::find_large_functions_tool` (tools/query.py:497).
//! Decomposition target finder — gives the LLM a fast pre-pass before
//! review or refactor planning. Schema v3 (`line_end` column) is required.
//!
//! Span metric: `line_end - line + 1`. Synthetic / metaprogramming symbols
//! with `line_end == line` register as 1-line spans and never trip the
//! default threshold.

use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LargeFunctionEntry {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub line: u32,
    pub line_end: u32,
    pub line_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LargeFunctionsMeta {
    pub total_matches: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LargeFunctionsResponse {
    pub functions: Vec<LargeFunctionEntry>,
    pub meta: LargeFunctionsMeta,
}

#[derive(Debug, Clone)]
pub struct LargeFunctionsRequest {
    /// Inclusive minimum line span. Default 50 — same as code-review-graph.
    pub min_lines: u32,
    /// Optional symbol-kind filter (`function`, `method`, `class`, ...).
    pub kind: Option<String>,
    /// Optional file-path substring filter (e.g. `src/utils/`).
    pub file_pattern: Option<String>,
    /// Result cap. Hard-capped at `LIMIT_CAP`.
    pub limit: u32,
}

impl Default for LargeFunctionsRequest {
    fn default() -> Self {
        Self {
            min_lines: 50,
            kind: None,
            file_pattern: None,
            limit: 50,
        }
    }
}

const LIMIT_CAP: u32 = 200;

pub fn large_functions(
    store: &Store,
    req: &LargeFunctionsRequest,
) -> Result<LargeFunctionsResponse> {
    let limit = req.limit.clamp(1, LIMIT_CAP) as usize;
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // Validate kind / file_pattern early — both flow into a Cypher
    // fragment so they get the same Tools-C9-d safe-input gate.
    if let Some(k) = req.kind.as_deref() {
        if !common::is_safe_ident(k) {
            return Ok(LargeFunctionsResponse::default());
        }
    }
    if let Some(p) = req.file_pattern.as_deref() {
        if p.contains('\'') || p.contains('\n') || p.contains('\r') {
            return Ok(LargeFunctionsResponse::default());
        }
    }

    // Pull all non-external symbols + line span. We filter & sort in Rust to
    // keep the Cypher portable across lbug versions (CONTAINS substring match
    // for file_pattern, plus the span arithmetic, are easier to read here).
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.file, s.kind, s.line, s.line_end")
        .map_err(|e| Error::Other(anyhow::anyhow!("large_functions query: {e}")))?;

    let mut matches: Vec<LargeFunctionEntry> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 5 {
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
            lbug::Value::Int64(n) if *n > 0 => *n as u32,
            _ => continue,
        };
        let line_end = match &cols[4] {
            lbug::Value::Int64(n) if *n > 0 => *n as u32,
            _ => line, // Schema-v2 cache leftover; treat as 1-line span.
        };

        // Apply user filters.
        if let Some(k) = req.kind.as_deref() {
            if kind != k {
                continue;
            }
        }
        if let Some(p) = req.file_pattern.as_deref() {
            if !file.contains(p) {
                continue;
            }
        }

        let line_count = line_end.saturating_sub(line).saturating_add(1);
        if line_count < req.min_lines {
            continue;
        }

        matches.push(LargeFunctionEntry {
            name,
            file,
            kind,
            line,
            line_end,
            line_count,
        });
    }

    // Sort descending by line_count, then file/name for stable ordering.
    matches.sort_by(|a, b| {
        b.line_count
            .cmp(&a.line_count)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.name.cmp(&b.name))
    });

    let total = matches.len() as u32;
    let truncated = matches.len() > limit;
    matches.truncate(limit);

    Ok(LargeFunctionsResponse {
        functions: matches,
        meta: LargeFunctionsMeta {
            total_matches: total,
            truncated,
        },
    })
}
