use anyhow::{anyhow, Result};
use ga_index::Store;
use ga_query::indexer::build_index;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn dump(conn: &lbug::Connection<'_>, label: &str, cypher: &str) -> Result<(usize, String)> {
    let rs = conn.query(cypher).map_err(|e| anyhow!("{label}: {e}"))?;
    let mut rows: Vec<String> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        let s: Vec<String> = cols
            .iter()
            .map(|v| match v {
                lbug::Value::String(s) => s.clone(),
                lbug::Value::Int64(n) => n.to_string(),
                lbug::Value::Int32(n) => n.to_string(),
                lbug::Value::Bool(b) => b.to_string(),
                lbug::Value::Null(_) => "<null>".to_string(),
                other => format!("{:?}", other),
            })
            .collect();
        rows.push(s.join("|"));
    }
    rows.sort();
    let joined = rows.join("\n");
    let mut h = Sha256::new();
    h.update(joined.as_bytes());
    let hash = format!("{:x}", h.finalize());
    Ok((rows.len(), hash))
}

fn main() -> Result<()> {
    let fixture = std::env::args()
        .nth(1)
        .expect("usage: snapshot_index <fixture>");
    let fixture = PathBuf::from(&fixture).canonicalize()?;
    let name = fixture.file_name().unwrap().to_string_lossy().to_string();

    let cache_root = tempfile::tempdir()?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache_root.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    let store = Store::open_with_root(cache_root.path(), &fixture)?;
    let stats = build_index(&store, &fixture)?;
    let conn = store.connection().map_err(|e| anyhow!("conn: {e}"))?;

    let queries: &[(&str, &str)] = &[
        ("File",       "MATCH (f:File) RETURN f.path, f.lang, f.size"),
        ("Symbol",     "MATCH (s:Symbol) RETURN s.id, s.name, s.file, s.kind, s.line"),
        ("DEFINES",    "MATCH (f:File)-[:DEFINES]->(s:Symbol) RETURN f.path, s.id"),
        ("CALLS",      "MATCH (a:Symbol)-[r:CALLS]->(b:Symbol) RETURN a.id, b.id, r.call_site_line"),
        ("IMPORTS",    "MATCH (a:File)-[r:IMPORTS]->(b:File) RETURN a.path, b.path, r.import_line, r.imported_names, r.re_export"),
        ("EXTENDS",    "MATCH (a:Symbol)-[:EXTENDS]->(b:Symbol) RETURN a.id, b.id"),
        ("REFERENCES", "MATCH (a:Symbol)-[r:REFERENCES]->(b:Symbol) RETURN a.id, b.id, r.ref_site_line, r.ref_kind"),
        ("TESTED_BY",  "MATCH (a:Symbol)-[:TESTED_BY]->(b:Symbol) RETURN a.id, b.id"),
        ("CONTAINS",   "MATCH (a:Symbol)-[:CONTAINS]->(b:Symbol) RETURN a.id, b.id"),
    ];

    println!("# snapshot fixture={name}");
    println!(
        "# IndexStats files={} symbols={} defines={} calls={} imports={} extends={} refs={}",
        stats.files,
        stats.symbols,
        stats.defines_edges,
        stats.calls_edges,
        stats.imports_edges,
        stats.extends_edges,
        stats.references_edges
    );
    for (label, cypher) in queries {
        let (count, hash) = dump(&conn, label, cypher)?;
        println!("{label} count={count} sha256={hash}");
    }
    Ok(())
}
