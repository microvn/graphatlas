//! Dump edge counts for a fixture — for line-by-line comparison vs CRG SQLite.
//!
//! Usage: cargo run -p ga-query --example dump_edge_counts -- <fixture-dir>

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::PathBuf;
use tempfile::TempDir;

fn main() {
    let fixture: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: dump_edge_counts <fixture>");
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache, &fixture).unwrap();
    let stats = build_index(&store, &fixture).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache, &fixture).unwrap();
    let conn = store.connection().unwrap();

    println!("=== ga indexed: {} ===", fixture.display());
    println!("files                        = {}", stats.files);
    println!("symbols                      = {}", stats.symbols);
    println!("defines_edges                = {}", stats.defines_edges);
    println!("calls_edges                  = {}", stats.calls_edges);
    println!("imports_edges                = {}", stats.imports_edges);
    println!("extends_edges                = {}", stats.extends_edges);
    println!("references_edges             = {}", stats.references_edges);
    println!(
        "module_typed_edges           = {}",
        stats.module_typed_edges
    );
    println!(
        "qualified_name_collision_count = {}",
        stats.qualified_name_collision_count
    );
    println!(
        "unresolved_imports_count     = {}",
        stats.unresolved_imports_count
    );
    println!(
        "unresolved_decorators_count  = {}",
        stats.unresolved_decorators_count
    );

    let n = |q: &str| -> i64 {
        let rs = conn.query(q).unwrap();
        for row in rs {
            if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
                return n;
            }
        }
        0
    };

    println!("\n=== REL row counts ===");
    for rel in [
        "DEFINES",
        "CONTAINS",
        "CALLS",
        "EXTENDS",
        "TESTED_BY",
        "REFERENCES",
        "MODULE_TYPED",
        "IMPORTS",
        "CALLS_HEURISTIC",
        "IMPLEMENTS",
        "IMPORTS_NAMED",
        "DECORATES",
    ] {
        let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
        println!("{:<20} = {}", rel, n(&q));
    }
}
