//! Tools S-003 cluster A — list files that import the given file.

use crate::{ImporterEntry, ImportersResponse};
use ga_core::{Error, Result};
use ga_index::Store;

/// Direct importers of `file`. AS-006 basic shape. Re-export / transitive
/// resolution land in clusters B and C.
pub fn importers(store: &Store, file: &str) -> Result<ImportersResponse> {
    // Tools-C9-d path safety — quote / newline rejection.
    if file.is_empty() || file.contains('\'') || file.contains('\n') || file.contains('\r') {
        return Ok(ImportersResponse::default());
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    let cypher = format!(
        "MATCH (src:File)-[r:IMPORTS]->(dst:File) \
         WHERE dst.path = '{}' \
         RETURN src.path, r.import_line, r.imported_names, r.re_export",
        file,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("importers query: {e}")))?;

    use std::collections::HashMap;
    let mut by_path: HashMap<String, ImporterEntry> = HashMap::new();

    // Pass 1 — direct importers.
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let path = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        if path == file {
            continue; // self-loop safety
        }
        let import_line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let imported_names = match &cols[2] {
            lbug::Value::String(s) if !s.is_empty() => {
                s.split('|').map(|t| t.to_string()).collect()
            }
            _ => Vec::new(),
        };
        let re_export = matches!(&cols[3], lbug::Value::Bool(true));
        by_path.insert(
            path.clone(),
            ImporterEntry {
                path,
                import_line,
                imported_names,
                re_export,
                via: None,
            },
        );
    }

    // Pass 2 — depth-2 transitive via a single re-export hop.
    add_transitive_depth2(&conn, file, &mut by_path)?;

    // Pass 3 — depth-3 transitive via two re-export hops.
    add_transitive_depth3(&conn, file, &mut by_path)?;

    let mut out: Vec<ImporterEntry> = by_path.into_values().collect();
    // Deterministic order by path for assertion-friendly output.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ImportersResponse { importers: out })
}

fn add_transitive_depth2(
    conn: &lbug::Connection<'_>,
    file: &str,
    by_path: &mut std::collections::HashMap<String, ImporterEntry>,
) -> Result<()> {
    // src -r1-> mid -r2-> dst (re_export). src→mid can be any kind.
    let cypher = format!(
        "MATCH (src:File)-[r1:IMPORTS]->(mid:File)-[r2:IMPORTS]->(dst:File) \
         WHERE dst.path = '{}' AND r2.re_export = true \
         RETURN src.path, r1.import_line, r1.imported_names, mid.path",
        file,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("importers depth-2 query: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let path = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        if path == file {
            continue;
        }
        if by_path.contains_key(&path) {
            continue; // direct wins, skip transitive dupes
        }
        let import_line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let imported_names = match &cols[2] {
            lbug::Value::String(s) if !s.is_empty() => {
                s.split('|').map(|t| t.to_string()).collect()
            }
            _ => Vec::new(),
        };
        let via = match &cols[3] {
            lbug::Value::String(s) => Some(s.clone()),
            _ => None,
        };
        by_path.insert(
            path.clone(),
            ImporterEntry {
                path,
                import_line,
                imported_names,
                re_export: true,
                via,
            },
        );
    }
    Ok(())
}

fn add_transitive_depth3(
    conn: &lbug::Connection<'_>,
    file: &str,
    by_path: &mut std::collections::HashMap<String, ImporterEntry>,
) -> Result<()> {
    // src -r1-> m1 -r2-> m2 -r3-> dst. r2 and r3 must be re_export.
    let cypher = format!(
        "MATCH (src:File)-[r1:IMPORTS]->(m1:File)-[r2:IMPORTS]->(m2:File)-[r3:IMPORTS]->(dst:File) \
         WHERE dst.path = '{}' AND r2.re_export = true AND r3.re_export = true \
         RETURN src.path, r1.import_line, r1.imported_names, m1.path",
        file,
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("importers depth-3 query: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let path = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        if path == file || by_path.contains_key(&path) {
            continue;
        }
        let import_line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let imported_names = match &cols[2] {
            lbug::Value::String(s) if !s.is_empty() => {
                s.split('|').map(|t| t.to_string()).collect()
            }
            _ => Vec::new(),
        };
        let via = match &cols[3] {
            lbug::Value::String(s) => Some(s.clone()),
            _ => None,
        };
        by_path.insert(
            path.clone(),
            ImporterEntry {
                path,
                import_line,
                imported_names,
                re_export: true,
                via,
            },
        );
    }
    Ok(())
}
