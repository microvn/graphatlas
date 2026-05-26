//! P3.1 (2026-05-22) — compact aggregation for callers/callees MCP responses.
//!
//! Pre-P3.1: GA returned one entry per call site. A single enclosing fn that
//! called the seed 5 times produced 5 entries with identical symbol/file/line
//! — repeat noise for LLM agents that just want "1 caller, 5 sites".
//!
//! P3.1 dedups at the MCP wrapper layer into one entry per
//! `(caller_symbol, file)` with `call_sites: [u32]` array + count. Internal
//! `ga_query::callers/callees` API stays per-site (bench retrievers / GT
//! generation / UI server depend on flat shape).
//!
//! Opt-out: `verbosity: "flat"` restores the per-site shape.

use ga_query::{CallKind, CalleeEntry, CallerEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Decide whether to apply compact aggregation. Default = compact. Opt-out
/// with `verbosity: "flat"` on the tool args. Unknown values silently fall
/// back to compact (forward-compatible).
pub(crate) fn wants_compact(args: &Value) -> bool {
    !args
        .get("verbosity")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("flat") || s.eq_ignore_ascii_case("full"))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompactCaller {
    pub symbol: String,
    pub file: String,
    /// Definition line of the enclosing caller function.
    pub line: u32,
    /// All call-site lines from this caller to the seed.
    pub call_sites: Vec<u32>,
    /// `call_sites.len()` — convenience for LLM scanning.
    pub call_site_count: u32,
    pub confidence: f32,
    pub kind: CallKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompactCallee {
    pub symbol: String,
    pub file: String,
    pub line: u32,
    pub call_sites: Vec<u32>,
    pub call_site_count: u32,
    pub confidence: f32,
    pub kind: CallKind,
    pub symbol_kind: String,
    pub external: bool,
}

pub(crate) fn compact_callers(entries: Vec<CallerEntry>) -> Vec<CompactCaller> {
    use std::collections::BTreeMap;
    // Key by (caller symbol, file, kind). `kind` separates direct CALL from
    // REFERENCES so a caller that both invokes and value-references the seed
    // shows up as two entries — preserves the AS-019 semantic distinction.
    let mut by_key: BTreeMap<(String, String, &'static str), CompactCaller> = BTreeMap::new();
    for c in entries {
        let kind_tag = match c.kind {
            CallKind::Call => "call",
            CallKind::Reference => "reference",
        };
        let key = (c.symbol.clone(), c.file.clone(), kind_tag);
        let agg = by_key.entry(key).or_insert_with(|| CompactCaller {
            symbol: c.symbol.clone(),
            file: c.file.clone(),
            line: c.line,
            call_sites: Vec::new(),
            call_site_count: 0,
            confidence: c.confidence,
            kind: c.kind,
        });
        if !agg.call_sites.contains(&c.call_site_line) {
            agg.call_sites.push(c.call_site_line);
        }
        // Keep lowest confidence — if any site is uncertain, the whole
        // aggregation inherits the lower bar.
        if c.confidence < agg.confidence {
            agg.confidence = c.confidence;
        }
    }
    let mut out: Vec<CompactCaller> = by_key
        .into_values()
        .map(|mut e| {
            e.call_sites.sort();
            e.call_site_count = e.call_sites.len() as u32;
            e
        })
        .collect();
    // Stable order: by file, then symbol, then kind.
    out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.symbol.cmp(&b.symbol)));
    out
}

pub(crate) fn compact_callees(entries: Vec<CalleeEntry>) -> Vec<CompactCallee> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<(String, String, &'static str), CompactCallee> = BTreeMap::new();
    for c in entries {
        let kind_tag = match c.kind {
            CallKind::Call => "call",
            CallKind::Reference => "reference",
        };
        let key = (c.symbol.clone(), c.file.clone(), kind_tag);
        let agg = by_key.entry(key).or_insert_with(|| CompactCallee {
            symbol: c.symbol.clone(),
            file: c.file.clone(),
            line: c.line,
            call_sites: Vec::new(),
            call_site_count: 0,
            confidence: c.confidence,
            kind: c.kind,
            symbol_kind: c.symbol_kind.clone(),
            external: c.external,
        });
        if !agg.call_sites.contains(&c.call_site_line) {
            agg.call_sites.push(c.call_site_line);
        }
        if c.confidence < agg.confidence {
            agg.confidence = c.confidence;
        }
    }
    let mut out: Vec<CompactCallee> = by_key
        .into_values()
        .map(|mut e| {
            e.call_sites.sort();
            e.call_site_count = e.call_sites.len() as u32;
            e
        })
        .collect();
    out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.symbol.cmp(&b.symbol)));
    out
}

/// Render a compact caller as a Markdown bullet line. Example:
///   - v1ClientIdRoutes (call) - apps/.../client-id.ts:8 (5 sites: 91, 117, ...)
pub(crate) fn render_compact_caller_md(c: &CompactCaller) -> String {
    let kind = match c.kind {
        CallKind::Call => "call",
        CallKind::Reference => "reference",
    };
    let sites_str = c
        .call_sites
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let conf_tag = if (c.confidence - 1.0).abs() > 1e-6 {
        format!(" [conf={:.1}]", c.confidence)
    } else {
        String::new()
    };
    if c.call_site_count == 1 {
        format!(
            "- {} ({}) - {}:{}{}",
            c.symbol, kind, c.file, c.call_sites[0], conf_tag
        )
    } else {
        format!(
            "- {} ({}) - {}:{} ({} sites: {}){}",
            c.symbol, kind, c.file, c.line, c.call_site_count, sites_str, conf_tag
        )
    }
}

pub(crate) fn render_compact_callee_md(c: &CompactCallee) -> String {
    let kind = match c.kind {
        CallKind::Call => "call",
        CallKind::Reference => "reference",
    };
    let sites_str = c
        .call_sites
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let ext = if c.external { ", external" } else { "" };
    if c.call_site_count == 1 {
        format!(
            "- {} ({}{}) - {}:{}",
            c.symbol, c.symbol_kind, ext, c.file, c.call_sites[0]
        )
    } else {
        let _ = kind;
        format!(
            "- {} ({}{}) - {}:{} ({} sites: {})",
            c.symbol, c.symbol_kind, ext, c.file, c.line, c.call_site_count, sites_str
        )
    }
}
