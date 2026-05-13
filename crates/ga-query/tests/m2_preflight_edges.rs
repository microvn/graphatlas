//! M2-00 pre-flight audit — TESTED_BY / REFERENCES / CALLS edge density
//! across real-OSS bench fixtures. Read-only research gate; not part of CI.
//!
//! Trigger: `GA_M2_EDGE_AUDIT=1 cargo test -p ga-query --release \
//!          --test m2_preflight_edges -- --nocapture --ignored`
//!
//! Writes a markdown table to `bench-results/m2-edge-density.md`. Use
//! output to decide whether M2-05 TESTED_BY chain Cypher moves the needle
//! per language (empty TESTED_BY on a fixture → chain query will 0-hit).

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn count(conn: &lbug::Connection<'_>, cypher: &str) -> i64 {
    let rs = conn.query(cypher).expect("query");
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if let Some(lbug::Value::Int64(n)) = cols.first() {
            return *n;
        }
    }
    -1
}

#[test]
#[ignore]
fn m2_edge_density_audit() {
    if std::env::var("GA_M2_EDGE_AUDIT").ok().as_deref() != Some("1") {
        eprintln!("skipped — set GA_M2_EDGE_AUDIT=1 to run");
        return;
    }

    let root = workspace_root();
    let fixtures_dir = root.join("benches/fixtures");
    // (name, lang-tag). Order smallest→largest so django runs last.
    let fixtures: &[(&str, &str)] = &[
        ("gin", "go"),
        ("axum", "rust"),
        ("preact", "ts/js"),
        ("nest", "ts/js"),
        ("django", "python"),
    ];

    let cache_root = root.join(".graphatlas-bench-cache/m2-audit");
    std::fs::create_dir_all(&cache_root).unwrap();

    let mut rows: Vec<(String, String, i64, i64, i64, f64)> = Vec::new();
    for (name, lang) in fixtures {
        let repo = fixtures_dir.join(name);
        if !repo.exists() {
            eprintln!("SKIP {name}: missing {}", repo.display());
            continue;
        }
        let t0 = std::time::Instant::now();
        eprintln!("=== {name} ({lang}) — indexing {}", repo.display());
        let store = Store::open_with_root(&cache_root, &repo).expect("open store");
        build_index(&store, &repo).expect("build index");
        let conn = store.connection().expect("conn");
        let tb = count(&conn, "MATCH ()-[t:TESTED_BY]->() RETURN count(t)");
        let rf = count(&conn, "MATCH ()-[r:REFERENCES]->() RETURN count(r)");
        let ca = count(&conn, "MATCH ()-[c:CALLS]->() RETURN count(c)");
        let elapsed = t0.elapsed().as_secs_f64();
        eprintln!("    TESTED_BY={tb}  REFERENCES={rf}  CALLS={ca}  ({elapsed:.1}s)");
        rows.push((name.to_string(), lang.to_string(), tb, rf, ca, elapsed));
    }

    let mut md = String::new();
    md.push_str("# M2-00 Pre-Flight — Edge Density Audit\n\n");
    md.push_str("| Fixture | Lang | TESTED_BY | REFERENCES | CALLS | Build (s) |\n");
    md.push_str("|---------|------|-----------|------------|-------|-----------|\n");
    for (name, lang, tb, rf, ca, sec) in &rows {
        md.push_str(&format!(
            "| {name} | {lang} | {tb} | {rf} | {ca} | {sec:.1} |\n"
        ));
    }
    let out = root.join("bench-results/m2-edge-density.md");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, &md).unwrap();
    eprintln!("\nwrote {}\n{md}", out.display());
}
