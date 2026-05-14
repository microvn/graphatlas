//! Tool query implementations. S-001 shipped response-shape types + stubs;
//! Tools S-001 (this milestone) starts filling in real queries + an indexer
//! pipeline to populate the graph they query.

pub mod architecture;
pub mod blame;
pub mod bridges;
pub mod callees;
pub mod common;
pub mod dead_code;
pub mod entry_points;
pub mod file_summary;
pub mod hubs;
pub mod impact;
pub mod import_resolve;
pub mod importers;
pub mod incremental;
pub mod indexer;
pub mod large_functions;
pub mod minimal_context;
mod phase_b;
pub mod psr4_resolve;
pub mod rename_safety;
pub mod risk;
pub mod signals;
pub mod snippet;
pub mod symbols;

pub use callees::callees;
pub use file_summary::file_summary;
pub use impact::{
    impact, AffectedConfig, AffectedRoute, AffectedTest, AffectedTestReason, BreakPoint,
    ImpactMeta, ImpactReason, ImpactRequest, ImpactResponse, ImpactedFile, Risk, RiskLevel,
    TotalAvailable, TruncationMeta,
};
pub use importers::importers;
pub use symbols::{symbols, SymbolsMatch};

use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};

/// AS-019 — distinguishes a direct call site (`"call"`) from a
/// value-reference site (`"reference"` — dispatch map, array element,
/// shorthand property). Introduced alongside REFERENCES edge per
/// Foundation-C15 / Tools-C16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    Call,
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerEntry {
    pub file: String,
    pub symbol: String,
    pub line: u32,
    pub call_site_line: u32,
    /// AS-003 polymorphic confidence. 1.0 when the callee's name is
    /// unambiguously resolved; 0.6 when the caller may be invoking a
    /// same-named def in a different file (static type unknown).
    pub confidence: f32,
    /// AS-019 — `"call"` for direct call-site; `"reference"` for
    /// value-reference site (dispatch map etc. via REFERENCES edge).
    pub kind: CallKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub score: f32,
}

/// Tools-C5 meta for ga_symbols. `truncated` + `total_available` expose what's
/// behind the 10-result cap so the LLM can decide whether to narrow the pattern.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolsMeta {
    pub truncated: bool,
    pub total_available: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolsResponse {
    pub symbols: Vec<SymbolEntry>,
    pub meta: SymbolsMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSummary {
    pub path: String,
    pub symbols: Vec<SymbolEntry>,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

/// AS-002 meta — tells the LLM whether the symbol was present in the graph
/// and (if absent) hands back Levenshtein-ranked suggestions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallersMeta {
    pub symbol_found: bool,
    pub suggestion: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallersResponse {
    pub callers: Vec<CallerEntry>,
    pub meta: CallersMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalleeEntry {
    pub file: String,
    pub symbol: String,
    /// Symbol kind: `"function"`, `"method"`, `"class"`, `"external"`, etc.
    /// Renamed from `kind` when AS-019 introduced the `kind: CallKind` field
    /// to disambiguate edge type (call vs reference).
    pub symbol_kind: String,
    pub line: u32,
    pub call_site_line: u32,
    pub confidence: f32,
    /// Cluster B — `true` when the callee is not defined in the indexed repo
    /// (stdlib / third-party / unresolved). Cluster A always emits `false`.
    #[serde(default)]
    pub external: bool,
    /// AS-019 — `"call"` for direct call-site; `"reference"` for
    /// value-reference site held by caller (REFERENCES edge).
    pub kind: CallKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalleesMeta {
    pub symbol_found: bool,
    pub suggestion: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalleesResponse {
    pub callees: Vec<CalleeEntry>,
    pub meta: CalleesMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImporterEntry {
    pub path: String,
    pub import_line: u32,
    /// Names pulled in by the importer. Cluster A leaves empty; filled in
    /// alongside re-export detection in cluster B.
    #[serde(default)]
    pub imported_names: Vec<String>,
    /// Cluster B — `true` when the entry is a transitive importer via
    /// `export * from '…'`.
    #[serde(default)]
    pub re_export: bool,
    /// Cluster C — intermediate file(s) the transitive chain passed through.
    #[serde(default)]
    pub via: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportersResponse {
    pub importers: Vec<ImporterEntry>,
}

/// Direct callers of `symbol`. If `file` is `Some`, restricts to callees
/// defined in that file (disambiguates same-name symbols across the repo).
///
/// Tools S-001 clusters C + D: happy-path query plus AS-002 not-found meta
/// with top-3 Levenshtein suggestions. Cross-file caller resolution lands
/// later once imports are indexed; for now this returns within-repo CALLS
/// edges written by the indexer.
pub fn callers(store: &Store, symbol: &str, file: Option<&str>) -> Result<CallersResponse> {
    // Tools-C9-d: identifier allowlist on query values. Non-ident input
    // short-circuits without touching the Cypher layer — AS-018.
    if !common::is_safe_ident(symbol) {
        return Ok(CallersResponse::default());
    }
    if let Some(f) = file {
        if f.contains('\'') || f.contains('\n') || f.contains('\r') {
            return Ok(CallersResponse::default());
        }
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // Drives AS-003 polymorphic confidence.
    let def_count = common::count_defs(&conn, symbol)?;

    // Always pull callee.file so we can classify each edge's confidence.
    // File filter is applied in Rust (not Cypher) because polymorphic
    // expansion deliberately includes other-file same-name defs.
    let cypher = format!(
        "MATCH (caller:Symbol)-[r:CALLS]->(callee:Symbol) \
         WHERE callee.name = '{}' \
         RETURN caller.file, caller.name, caller.line, r.call_site_line, callee.file",
        symbol,
    );

    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("callers query: {e}")))?;

    let mut callers_out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 5 {
            continue;
        }
        let caller_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let caller_name = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let call_site_line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let callee_file = match &cols[4] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };

        let confidence = match (def_count, file) {
            (n, _) if n <= 1 => 1.0,                 // single def — unambiguous
            (_, Some(f)) if callee_file == f => 1.0, // exact match on filter
            _ => 0.6,                                // polymorphic / ambiguous
        };

        // Without a file filter + single def → return everything.
        // With a file filter + single def → keep only callers hitting that
        // def's file (matches cluster C strict-filter semantics).
        if def_count <= 1 {
            if let Some(f) = file {
                if callee_file != f {
                    continue;
                }
            }
        }

        callers_out.push(CallerEntry {
            file: caller_file,
            symbol: caller_name,
            line,
            call_site_line,
            confidence,
            kind: CallKind::Call,
        });
    }

    // Tools-C16 — value-reference callers via REFERENCES edge. Same
    // polymorphic-confidence rule applies. Merged into the same list with
    // kind: Reference to distinguish from direct CALLS.
    let refs_cypher = format!(
        "MATCH (caller:Symbol)-[r:REFERENCES]->(target:Symbol) \
         WHERE target.name = '{}' \
         RETURN caller.file, caller.name, caller.line, r.ref_site_line, target.file",
        symbol,
    );
    let rs = conn
        .query(&refs_cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("references query: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 5 {
            continue;
        }
        let caller_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let caller_name = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let ref_site_line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let target_file = match &cols[4] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let confidence = match (def_count, file) {
            (n, _) if n <= 1 => 1.0,
            (_, Some(f)) if target_file == f => 1.0,
            _ => 0.6,
        };
        if def_count <= 1 {
            if let Some(f) = file {
                if target_file != f {
                    continue;
                }
            }
        }
        callers_out.push(CallerEntry {
            file: caller_file,
            symbol: caller_name,
            line,
            call_site_line: ref_site_line,
            confidence,
            kind: CallKind::Reference,
        });
    }

    // Determine symbol_found — may be true even when callers_out is empty
    // (symbol exists but nobody calls it, e.g. an entry point).
    let symbol_found = common::symbol_exists(&conn, symbol)?;
    let suggestion = if symbol_found {
        Vec::new()
    } else {
        common::suggest_similar(&conn, symbol)
    };

    Ok(CallersResponse {
        callers: callers_out,
        meta: CallersMeta {
            symbol_found,
            suggestion,
        },
    })
}
