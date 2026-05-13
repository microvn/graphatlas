//! v1.3 PR5c spike — empirical investigation of lbug 0.16.1 composite
//! column behavior for `params STRUCT(...)[]` and `modifiers STRING[]`.
//!
//! Goal: answer 5 unknowns before committing PR5c production code:
//!
//! Q1. lbug CSV LIST<STRING> syntax: how to write `[]` and `["a","b"]` in
//!     a CSV cell that COPY parses correctly?
//! Q2. lbug CSV STRUCT(...)[] syntax: how to write `[]` and
//!     `[{name:'x',type:'i32',default_value:''}]`?
//! Q3. Does CREATE NODE TABLE with composite columns (NO DEFAULT) +
//!     full-row CSV emission survive empty-cache reopen lifecycle? (Path G
//!     for composites, untested by Tools-C13 baseline)
//! Q4. Does CREATE NODE TABLE with composite DEFAULT (e.g.
//!     `DEFAULT CAST([] AS ...)`) trigger kuzu#6045 OverflowFile assertion
//!     on test-harness reopen of empty cache?
//! Q5. If both Q3 and Q4 fail: what working path remains for PR5c?
//!
//! Run via:
//!   LBUG_BUILD_FROM_SOURCE=1 cargo run -p ga-index --example spike_pr5c_composites
//!
//! Output: PASS/FAIL per scenario + recommended PR5c implementation path.

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;
use tempfile::TempDir;

fn write_csv(dir: &TempDir, name: &str, rows: &[&str]) -> std::path::PathBuf {
    let p = dir.path().join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    for r in rows {
        writeln!(f, "{r}").unwrap();
    }
    p
}

fn try_query(conn: &Connection, label: &str, q: &str) -> bool {
    match conn.query(q) {
        Ok(_) => {
            println!("  ✅ {label}");
            true
        }
        Err(e) => {
            // Truncate long error messages
            let msg = format!("{e}");
            let short = msg.lines().next().unwrap_or(&msg);
            println!("  ❌ {label}: {short}");
            false
        }
    }
}

fn show_rows(conn: &Connection, q: &str) {
    match conn.query(q) {
        Ok(rs) => {
            for row in rs {
                let cells: Vec<String> = row.into_iter().map(|v| format!("{v:?}")).collect();
                println!("    row: [{}]", cells.join(", "));
            }
        }
        Err(e) => println!("    query err: {e}"),
    }
}

fn main() {
    println!("=== PR5c spike: lbug 0.16.1 composite column behavior ===\n");

    // ────────────────────────────────────────────────────────────────────
    // Q1 — LIST<STRING> CSV syntax variants
    // ────────────────────────────────────────────────────────────────────
    println!("Q1. LIST<STRING> CSV syntax — find a writeln format that COPY parses\n");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("q1.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, mods STRING[])")
            .unwrap();

        // Variant A: bracket literal `[]` and `[a,b]`
        let csv = write_csv(&dir, "v_a.csv", &["a1,[]", "a2,[pub]", "a3,[pub,async]"]);
        let ok = try_query(
            &conn,
            "Variant A — `[]`, `[pub]`, `[pub,async]`",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
        if ok {
            show_rows(&conn, "MATCH (t:T) RETURN t.id, t.mods");
        }

        conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
        // Variant B: quoted bracket `"[]"` and `"[\"a\",\"b\"]"`
        let csv = write_csv(
            &dir,
            "v_b.csv",
            &["b1,\"[]\"", "b2,\"[pub]\"", "b3,\"[pub,async]\""],
        );
        let ok = try_query(
            &conn,
            "Variant B — quoted `\"[]\"`, `\"[pub]\"`",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
        if ok {
            show_rows(&conn, "MATCH (t:T) RETURN t.id, t.mods");
        }

        conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
        // Variant C: pipe-separated semicolon style
        let csv = write_csv(&dir, "v_c.csv", &["c1,\"\"", "c2,pub", "c3,pub|async"]);
        let _ = try_query(
            &conn,
            "Variant C — empty/single/pipe-separated (probably wrong)",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
    }

    // ────────────────────────────────────────────────────────────────────
    // Q2 — STRUCT(...)[] CSV syntax variants
    // ────────────────────────────────────────────────────────────────────
    println!("\nQ2. STRUCT(name,type,default_value)[] CSV syntax\n");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("q2.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query(
            "CREATE NODE TABLE T(id STRING PRIMARY KEY, \
             params STRUCT(name STRING, type STRING, default_value STRING)[])",
        )
        .unwrap();

        // Variant A: `[]` empty + `[{name:x,type:i32,default_value:}]`
        let csv = write_csv(
            &dir,
            "p_a.csv",
            &["a1,[]", "a2,[{name: x, type: i32, default_value: }]"],
        );
        let ok = try_query(
            &conn,
            "Variant A — bracket literal with inline struct",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
        if ok {
            show_rows(&conn, "MATCH (t:T) RETURN t.id, size(t.params)");
        }

        conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
        // Variant B: quoted version
        let csv = write_csv(
            &dir,
            "p_b.csv",
            &[
                "b1,\"[]\"",
                "b2,\"[{name: x, type: i32, default_value: }]\"",
            ],
        );
        let ok = try_query(
            &conn,
            "Variant B — quoted struct list",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
        if ok {
            show_rows(&conn, "MATCH (t:T) RETURN t.id, size(t.params)");
        }

        conn.query("MATCH (t:T) DETACH DELETE t").unwrap();
        // Variant C: empty cell (NULL → maybe parses as empty list?)
        let csv = write_csv(&dir, "p_c.csv", &["c1,"]);
        let ok = try_query(
            &conn,
            "Variant C — empty CSV cell (parses as null/empty?)",
            &format!("COPY T FROM '{}' (header=false)", csv.display()),
        );
        if ok {
            show_rows(
                &conn,
                "MATCH (t:T) RETURN t.id, size(t.params), t.params IS NULL",
            );
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Q3 — CREATE NODE TABLE w/ composite cols (NO DEFAULT) + reopen
    // ────────────────────────────────────────────────────────────────────
    println!("\nQ3. CREATE-no-DEFAULT composites + empty-cache reopen (Path G adapted)\n");
    {
        let dir = TempDir::new().unwrap();
        // Round 1: CREATE schema, COMMIT empty (no rows), CLOSE
        {
            let db = Database::new(dir.path().join("q3.db"), SystemConfig::default()).unwrap();
            let conn = Connection::new(&db).unwrap();
            let ok = try_query(
                &conn,
                "CREATE NODE TABLE w/ STRUCT[] + STRING[] (NO DEFAULT)",
                "CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING, \
                 mods STRING[], params STRUCT(n STRING, t STRING)[])",
            );
            if !ok {
                println!("  STOP — DDL failed at round 1");
                return;
            }
            // Don't insert anything — test the empty-cache reopen pattern
            // exactly as PR2 found broke kuzu#6045
            drop(conn);
            drop(db);
            println!("  Round 1 closed (empty cache)");
        }
        // Round 2: REOPEN — does kuzu#6045 trigger?
        {
            match std::panic::catch_unwind(|| {
                let db = Database::new(dir.path().join("q3.db"), SystemConfig::default())
                    .expect("reopen Database");
                let conn = Connection::new(&db).expect("reopen Connection");
                conn.query("MATCH (t:T) RETURN count(t)")
                    .expect("post-reopen query");
            }) {
                Ok(_) => {
                    println!("  ✅ Round 2 reopen OK — kuzu#6045 NOT triggered (no DEFAULT path)")
                }
                Err(_) => println!("  ❌ Round 2 reopen PANIC — kuzu#6045 triggered"),
            }
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Q4 — CREATE NODE TABLE w/ composite DEFAULT + reopen
    // ────────────────────────────────────────────────────────────────────
    println!("\nQ4. CREATE-with-DEFAULT composites + empty-cache reopen (PR2-trap path)\n");
    {
        let dir = TempDir::new().unwrap();
        {
            let db = Database::new(dir.path().join("q4.db"), SystemConfig::default()).unwrap();
            let conn = Connection::new(&db).unwrap();
            let ok = try_query(
                &conn,
                "CREATE NODE TABLE w/ STRING[] DEFAULT [] + STRUCT[] DEFAULT CAST([] AS ...)",
                "CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING, \
                 mods STRING[] DEFAULT CAST([] AS STRING[]), \
                 params STRUCT(n STRING, t STRING)[] \
                 DEFAULT CAST([] AS STRUCT(n STRING, t STRING)[]))",
            );
            if !ok {
                println!("  STOP — DDL failed at round 1");
                return;
            }
            drop(conn);
            println!("  Round 1 closed (empty cache, with DEFAULTs)");
        }
        {
            match std::panic::catch_unwind(|| {
                let db = Database::new(dir.path().join("q4.db"), SystemConfig::default())
                    .expect("reopen Database");
                let conn = Connection::new(&db).expect("reopen Connection");
                conn.query("MATCH (t:T) RETURN count(t)")
                    .expect("post-reopen query");
            }) {
                Ok(_) => {
                    println!("  ✅ Round 2 reopen OK — kuzu#6045 NOT triggered with DEFAULT path")
                }
                Err(_) => println!(
                    "  ❌ Round 2 reopen PANIC — kuzu#6045 triggered (PR2 finding confirmed)"
                ),
            }
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Q5 — full lifecycle: composite cols (NO DEFAULT) + populate + reopen
    // ────────────────────────────────────────────────────────────────────
    println!("\nQ5. Full lifecycle: NO-DEFAULT composites + populated COPY + reopen\n");
    {
        let dir = TempDir::new().unwrap();
        {
            let db = Database::new(dir.path().join("q5.db"), SystemConfig::default()).unwrap();
            let conn = Connection::new(&db).unwrap();
            conn.query(
                "CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING, \
                 mods STRING[], params STRUCT(n STRING, t STRING)[])",
            )
            .unwrap();
            // Populate via Cypher CREATE (we'll learn CSV syntax separately)
            try_query(
                &conn,
                "Cypher CREATE w/ inline composite literals",
                "CREATE (:T {id: 'x', name: 'foo', mods: ['pub','async'], \
                 params: [{n: 'a', t: 'i32'}, {n: 'b', t: 'String'}]})",
            );
            show_rows(&conn, "MATCH (t:T) RETURN t.id, t.mods, size(t.params)");
            drop(conn);
        }
        {
            match std::panic::catch_unwind(|| {
                let db = Database::new(dir.path().join("q5.db"), SystemConfig::default())
                    .expect("reopen");
                let conn = Connection::new(&db).expect("conn");
                let rs = conn
                    .query("MATCH (t:T) RETURN t.id, t.mods, size(t.params)")
                    .expect("query");
                let collected: Vec<Vec<String>> = rs
                    .map(|row| row.into_iter().map(|v| format!("{v:?}")).collect())
                    .collect();
                collected
            }) {
                Ok(rows) => {
                    println!("  ✅ Round 2 reopen OK — populated cache survives");
                    for cells in rows {
                        println!("    {}", cells.join(" | "));
                    }
                }
                Err(_) => println!("  ❌ Round 2 reopen PANIC even with populated rows"),
            }
        }
    }

    println!("\n=== PR5c spike done ===");
    println!("\nAdjudication checklist:");
    println!("  - If Q1+Q2 found a working CSV syntax → COPY-driven emission viable");
    println!("  - Else → must use Cypher CREATE per-row (slow, only OK for small repos)");
    println!("  - If Q3 ✅ AND Q4 ❌ → confirmed: ship NO-DEFAULT in BASE_DDL");
    println!("  - If Q3 ❌ → all composite paths trigger kuzu#6045; must defer PR5c");
    println!("  - If Q5 ✅ → full lifecycle works; PR5c production path identified");
}
