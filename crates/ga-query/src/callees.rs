//! Tools S-002 — outgoing callees of a symbol.

use crate::common::{count_defs, is_safe_ident, suggest_similar, symbol_exists};
use crate::{CallKind, CalleeEntry, CalleesMeta, CalleesResponse};
use ga_core::{Error, Result};
use ga_index::Store;

/// Outgoing callees from `symbol`. Same narrowing-hint semantic as
/// [`crate::callers`] (Tools-C11): `file` disambiguates the CALLER def;
/// callees of same-named callers in OTHER files come back with confidence 0.6.
///
/// Tools S-002 cluster A — within-repo CALLS edges only. External callees
/// (stdlib, third-party) land in cluster B with `external: true`.
pub fn callees(store: &Store, symbol: &str, file: Option<&str>) -> Result<CalleesResponse> {
    if !is_safe_ident(symbol) {
        return Ok(CalleesResponse::default());
    }
    if let Some(f) = file {
        if f.contains('\'') || f.contains('\n') || f.contains('\r') {
            return Ok(CalleesResponse::default());
        }
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    let def_count = count_defs(&conn, symbol)?;

    let cypher = format!(
        "MATCH (caller:Symbol)-[r:CALLS]->(callee:Symbol) \
         WHERE caller.name = '{}' \
         RETURN callee.file, callee.name, callee.kind, callee.line, r.call_site_line, caller.file",
        symbol,
    );

    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("callees query: {e}")))?;

    let mut callees_out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 6 {
            continue;
        }
        let callee_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let callee_name = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let kind = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => String::from("other"),
        };
        let line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let call_site_line = match &cols[4] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let caller_file = match &cols[5] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };

        let confidence = match (def_count, file) {
            (n, _) if n <= 1 => 1.0,
            (_, Some(f)) if caller_file == f => 1.0,
            _ => 0.6,
        };

        if def_count <= 1 {
            if let Some(f) = file {
                if caller_file != f {
                    continue;
                }
            }
        }

        let external = kind == "external";
        callees_out.push(CalleeEntry {
            file: callee_file,
            symbol: callee_name,
            symbol_kind: kind,
            line,
            call_site_line,
            confidence,
            external,
            kind: CallKind::Call,
        });
    }

    // Tools-C16 — value-reference callees via REFERENCES edge.
    let refs_cypher = format!(
        "MATCH (caller:Symbol)-[r:REFERENCES]->(target:Symbol) \
         WHERE caller.name = '{}' \
         RETURN target.file, target.name, target.kind, target.line, r.ref_site_line, caller.file",
        symbol,
    );
    let rs = conn
        .query(&refs_cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("references query: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 6 {
            continue;
        }
        let target_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let target_name = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let target_kind = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => String::from("other"),
        };
        let line = match &cols[3] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let ref_site_line = match &cols[4] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let caller_file = match &cols[5] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let confidence = match (def_count, file) {
            (n, _) if n <= 1 => 1.0,
            (_, Some(f)) if caller_file == f => 1.0,
            _ => 0.6,
        };
        if def_count <= 1 {
            if let Some(f) = file {
                if caller_file != f {
                    continue;
                }
            }
        }
        let external = target_kind == "external";
        callees_out.push(CalleeEntry {
            file: target_file,
            symbol: target_name,
            symbol_kind: target_kind,
            line,
            call_site_line: ref_site_line,
            confidence,
            external,
            kind: CallKind::Reference,
        });
    }

    let symbol_found = symbol_exists(&conn, symbol)?;
    let suggestion = if symbol_found {
        Vec::new()
    } else {
        suggest_similar(&conn, symbol)
    };

    Ok(CalleesResponse {
        callees: callees_out,
        meta: CalleesMeta {
            symbol_found,
            suggestion,
        },
    })
}
