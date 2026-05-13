//! Tools S-005 — ga_file_summary. Outline of a file: symbols defined inside
//! (ordered by line), imports (repo-local dst paths from IMPORTS edges), and
//! exports (symbol names; per-lang visibility isn't tracked in the graph yet).

use crate::{FileSummary, SymbolEntry};
use ga_core::{Error, Result};
use ga_index::Store;

/// AS-011 — returns a FileSummary for `path`. An unknown path produces an
/// empty summary (not an error) so the LLM can fall through without a retry.
pub fn file_summary(store: &Store, path: &str) -> Result<FileSummary> {
    if path.is_empty() || path.contains('\'') || path.contains('\n') || path.contains('\r') {
        return Ok(FileSummary {
            path: path.to_string(),
            symbols: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
        });
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    let symbols = query_file_symbols(&conn, path)?;
    let imports = query_file_imports(&conn, path)?;
    // Exports: no per-lang visibility data in the graph today, so surface the
    // names of everything defined in the file. Shape-compatible with the spec
    // and useful for blast-radius prompts.
    let exports = symbols.iter().map(|s| s.name.clone()).collect();

    Ok(FileSummary {
        path: path.to_string(),
        symbols,
        imports,
        exports,
    })
}

fn query_file_symbols(conn: &lbug::Connection<'_>, path: &str) -> Result<Vec<SymbolEntry>> {
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.file = '{}' AND s.kind <> 'external' \
         RETURN s.name, s.kind, s.line",
        path,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("file-summary symbols: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 3 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let kind = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => String::from("other"),
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        out.push(SymbolEntry {
            name,
            kind,
            file: path.to_string(),
            line,
            score: 1.0,
        });
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn query_file_imports(conn: &lbug::Connection<'_>, path: &str) -> Result<Vec<String>> {
    let cypher = format!(
        "MATCH (src:File)-[:IMPORTS]->(dst:File) WHERE src.path = '{}' RETURN dst.path",
        path,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("file-summary imports: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.push(s);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
