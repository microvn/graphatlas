//! Test the REAL v1.3 migration lifecycle (not the synthetic Path F).
//!
//! The actual v3 → v4 ALTER migration scenario that PR2 would implement:
//!   1. v3 cache exists, has populated rows from prior session
//!   2. v4 binary opens cache → ALTER ADD v4 cols with DOUBLE-DEFAULT (kuzu#5159 workaround)
//!   3. ALTER applies DEFAULT to existing rows
//!   4. User re-indexes → indexer DETACH DELETEs all rows, then COPY-column-list-omit
//!
//! Path F earlier used FLOAT (which trips kuzu#5159). This test uses DOUBLE
//! per Tools-C12 — same workaround that makes PR1 CREATE-with-DEFAULT work.
//! If DOUBLE works through the FULL ALTER-then-reindex lifecycle, PR2's
//! ALTER-incremental thesis is VIABLE.

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;
use tempfile::TempDir;

fn run_test(name: &str, body: impl FnOnce(&Connection, &TempDir)) {
    println!("\n=== {name} ===");
    let dir = TempDir::new().unwrap();
    let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
    let conn = Connection::new(&db).unwrap();
    body(&conn, &dir);
}

fn write_csv(dir: &TempDir, name: &str, rows: &[&str]) -> std::path::PathBuf {
    let p = dir.path().join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    for r in rows {
        writeln!(f, "{r}").unwrap();
    }
    drop(f);
    p
}

fn show_rows(conn: &Connection, q: &str) {
    match conn.query(q) {
        Ok(rs) => {
            for row in rs {
                let cells: Vec<_> = row.into_iter().collect();
                println!("  row: {cells:?}");
            }
        }
        Err(e) => println!("  query FAIL: {e}"),
    }
}

fn main() {
    // ==================================================================
    // Scenario H: ALTER ADD DOUBLE on populated table, then re-index
    //   (real v1.3 migration lifecycle — Tools-C12 DOUBLE workaround)
    // ==================================================================
    run_test(
        "H — ALTER ADD DOUBLE → DELETE → COPY column-list-omit (real reindex lifecycle)",
        |conn, dir| {
            // Step 1: CREATE v3-shape
            conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
                .unwrap();
            // Step 2: COPY v3 rows (existing cache state)
            let csv1 = write_csv(dir, "v3.csv", &["a,foo", "b,bar", "c,baz"]);
            conn.query(&format!("COPY T FROM '{}' (header=false)", csv1.display()))
                .unwrap();
            println!("  after v3 COPY:");
            show_rows(conn, "MATCH (t:T) RETURN t.id, t.name");
            // Step 3: ALTER ADD v4 col (DOUBLE — Tools-C12 workaround)
            match conn.query("ALTER TABLE T ADD confidence DOUBLE DEFAULT 1.0") {
                Ok(_) => println!("  ALTER ADD DOUBLE OK"),
                Err(e) => {
                    println!("  ALTER ADD DOUBLE FAIL: {e}");
                    return;
                }
            }
            println!("  after ALTER (existing rows should have DEFAULT 1.0):");
            show_rows(conn, "MATCH (t:T) RETURN t.id, t.name, t.confidence");
            // Step 4: User re-indexes → DETACH DELETE all rows
            conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
            println!("  after DELETE: rows = {}", count_rows(conn));
            // Step 5: COPY column-list omitting the v4 confidence col
            let csv2 = write_csv(dir, "v4.csv", &["x,xxx", "y,yyy"]);
            match conn.query(&format!(
                "COPY T (id, name) FROM '{}' (header=false)",
                csv2.display()
            )) {
                Ok(_) => {
                    println!("  ✅ COPY column-list-omit OK after ALTER+DELETE");
                    show_rows(conn, "MATCH (t:T) RETURN t.id, t.name, t.confidence");
                }
                Err(e) => println!("  ❌ COPY FAIL: {e}"),
            }
        },
    );

    // ==================================================================
    // Scenario H2: same but with FLOAT to confirm kuzu#5159 still bites
    // ==================================================================
    run_test(
        "H2 — ALTER ADD FLOAT (NOT DOUBLE) → DELETE → COPY column-list-omit",
        |conn, dir| {
            conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
                .unwrap();
            let csv1 = write_csv(dir, "v3.csv", &["a,foo"]);
            conn.query(&format!("COPY T FROM '{}' (header=false)", csv1.display()))
                .unwrap();
            match conn.query("ALTER TABLE T ADD confidence FLOAT DEFAULT 1.0") {
                Ok(_) => println!("  ALTER ADD FLOAT OK (DDL only — bug fires later at COPY)"),
                Err(e) => {
                    println!("  ALTER ADD FLOAT FAIL: {e}");
                    return;
                }
            }
            conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
            let csv2 = write_csv(dir, "v4.csv", &["x,xxx"]);
            match conn.query(&format!(
                "COPY T (id, name) FROM '{}' (header=false)",
                csv2.display()
            )) {
                Ok(_) => println!("  ⚠️  COPY OK with FLOAT — kuzu#5159 doesn't bite here?!"),
                Err(e) => println!("  ❌ COPY FAIL (expected — kuzu#5159 family): {e}"),
            }
        },
    );

    // ==================================================================
    // Scenario I: even simpler — multiple successive COPY column-list-omit
    //   on PR1 final shape (CREATE-with-DEFAULT all in BASE_DDL)
    // ==================================================================
    run_test(
        "I — CREATE-with-DOUBLE-DEFAULT, repeated DELETE+COPY (PR1 lifecycle confirm)",
        |conn, dir| {
            conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING, confidence DOUBLE DEFAULT 1.0)").unwrap();
            for round in 1..=3 {
                let csv = write_csv(
                    dir,
                    &format!("r{round}.csv"),
                    &[&format!("a{round},foo{round}")],
                );
                match conn.query(&format!(
                    "COPY T (id, name) FROM '{}' (header=false)",
                    csv.display()
                )) {
                    Ok(_) => println!("  round {round}: COPY column-list-omit OK"),
                    Err(e) => {
                        println!("  round {round}: COPY FAIL: {e}");
                        return;
                    }
                }
                if round < 3 {
                    conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
                }
            }
            println!("  final state:");
            show_rows(conn, "MATCH (t:T) RETURN t.id, t.confidence");
        },
    );

    // ==================================================================
    // Scenario J: ALTER ADD STRUCT[] DEFAULT in real migration lifecycle
    // ==================================================================
    run_test(
        "J — ALTER ADD STRUCT[] DEFAULT → DELETE → COPY column-list-omit (composite migration)",
        |conn, dir| {
            conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
                .unwrap();
            let csv1 = write_csv(dir, "v3.csv", &["a,foo", "b,bar"]);
            conn.query(&format!("COPY T FROM '{}' (header=false)", csv1.display()))
                .unwrap();
            match conn.query(
                "ALTER TABLE T ADD params STRUCT(name STRING, type STRING)[] \
             DEFAULT CAST([] AS STRUCT(name STRING, type STRING)[])",
            ) {
                Ok(_) => println!("  ALTER ADD STRUCT[] OK"),
                Err(e) => {
                    println!("  ALTER ADD STRUCT[] FAIL: {e}");
                    return;
                }
            }
            println!("  after ALTER:");
            show_rows(conn, "MATCH (t:T) RETURN t.id, t.name, size(t.params)");
            conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
            let csv2 = write_csv(dir, "v4.csv", &["x,xxx"]);
            match conn.query(&format!(
                "COPY T (id, name) FROM '{}' (header=false)",
                csv2.display()
            )) {
                Ok(_) => {
                    println!("  ✅ COPY column-list-omit OK with STRUCT[] DEFAULT");
                    show_rows(conn, "MATCH (t:T) RETURN t.id, size(t.params)");
                }
                Err(e) => println!("  ❌ COPY FAIL: {e}"),
            }
        },
    );

    // ==================================================================
    // Scenario K: same with LIST<STRING>
    // ==================================================================
    run_test(
        "K — ALTER ADD LIST<STRING> DEFAULT → DELETE → COPY column-list-omit",
        |conn, dir| {
            conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
                .unwrap();
            let csv1 = write_csv(dir, "v3.csv", &["a,foo"]);
            conn.query(&format!("COPY T FROM '{}' (header=false)", csv1.display()))
                .unwrap();
            match conn.query("ALTER TABLE T ADD modifiers STRING[] DEFAULT CAST([] AS STRING[])") {
                Ok(_) => println!("  ALTER ADD LIST<STRING> OK"),
                Err(e) => {
                    println!("  ALTER ADD LIST<STRING> FAIL: {e}");
                    return;
                }
            }
            conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
            let csv2 = write_csv(dir, "v4.csv", &["x,xxx"]);
            match conn.query(&format!(
                "COPY T (id, name) FROM '{}' (header=false)",
                csv2.display()
            )) {
                Ok(_) => {
                    println!("  ✅ COPY column-list-omit OK with LIST<STRING> DEFAULT");
                    show_rows(conn, "MATCH (t:T) RETURN t.id, size(t.modifiers)");
                }
                Err(e) => println!("  ❌ COPY FAIL: {e}"),
            }
        },
    );
}

fn count_rows(conn: &Connection) -> i64 {
    if let Ok(rs) = conn.query("MATCH (t:T) RETURN count(t)") {
        for row in rs {
            if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
                return n;
            }
        }
    }
    -1
}
