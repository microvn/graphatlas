//! Diag: scan Symbol rows for duplicates by (file, name, line) composite —
//! the key the ga-ui search dropdown / sidebar tree uses.

use ga_index::Store;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    let cache_root = PathBuf::from(std::env::var("HOME").unwrap()).join(".graphatlas");
    let repo_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: diag_duplicate_symbols <repo_root>");

    let store = Store::open_with_root(&cache_root, &repo_root).unwrap();
    let conn = store.connection().unwrap();

    let q = "MATCH (s:Symbol) WHERE s.kind <> 'external' \
             RETURN s.id, s.name, s.file, s.line, s.kind, s.qualified_name";
    let rs = conn.query(q).unwrap();

    let mut by_composite: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    let mut total = 0u64;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 6 {
            continue;
        }
        let id = string_or(&cols[0]);
        let name = string_or(&cols[1]);
        let file = string_or(&cols[2]);
        let line = int_or(&cols[3]);
        let kind = string_or(&cols[4]);
        let qname = string_or(&cols[5]);
        let composite = format!("{file}::{name}:{line}");
        by_composite
            .entry(composite)
            .or_default()
            .push((id, kind, qname));
        total += 1;
    }

    let mut dups: Vec<_> = by_composite.iter().filter(|(_, v)| v.len() > 1).collect();
    dups.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));

    println!("Total Symbol rows: {total}");
    println!("Distinct composites (file::name:line): {}", by_composite.len());
    println!("Duplicate groups: {}", dups.len());
    println!(
        "Extra rows (sum of count-1): {}",
        dups.iter().map(|(_, v)| v.len() as i64 - 1).sum::<i64>()
    );
    println!("\nTop 20 duplicates:");
    for (k, v) in dups.iter().take(20) {
        println!("  {} × {}", v.len(), k);
        for (id, kind, qname) in v.iter().take(5) {
            println!("      id={id}  kind={kind}  qname='{qname}'");
        }
    }
}

fn string_or(v: &lbug::Value) -> String {
    match v {
        lbug::Value::String(s) => s.clone(),
        _ => String::new(),
    }
}
fn int_or(v: &lbug::Value) -> i64 {
    match v {
        lbug::Value::Int64(n) => *n,
        _ => 0,
    }
}
