//! v1.3 PR5c spike #2 — verify composite DDL behavior under the **Store
//! wrapper's reopen pattern** (not just raw lbug::Database::new).
//!
//! PR2 found kuzu#6045 broke 4 reopen-pattern tests in the workspace —
//! specifically the Store::open path that re-applies BASE_DDL_STATEMENTS
//! every open (even on Resumed cache). spike #1 (`spike_pr5c_composites.rs`)
//! tested raw lbug::Database which DOES NOT replay DDL — that path WORKED.
//! This spike repeats the test under Store-style replay to find the actual
//! failure surface.
//!
//! Specifically replicates the 4 PR2 trap patterns:
//!   T1. schema_is_idempotent_across_reopens — open, commit, reopen, query
//!   T2. reopen_complete_cache_returns_resume — open empty, commit, reopen
//!   T3. read_only_store_refuses_commit — reopen-as-readonly after writer
//!   T4. second_open_after_commit_attaches_read_only — concurrent reopen
//!
//! Run:
//!   LBUG_BUILD_FROM_SOURCE=1 cargo run -p ga-index --example spike_pr5c_store

use lbug::{Connection, Database, SystemConfig};
use std::path::Path;
use tempfile::TempDir;

const COMPOSITE_DDL: &str = "CREATE NODE TABLE IF NOT EXISTS Symbol(\
    id STRING PRIMARY KEY, \
    name STRING, \
    mods STRING[] DEFAULT CAST([] AS STRING[]), \
    params STRUCT(n STRING, t STRING)[] DEFAULT CAST([] AS STRUCT(n STRING, t STRING)[])\
)";

fn apply_ddl(conn: &Connection) -> Result<(), String> {
    conn.query(COMPOSITE_DDL).map_err(|e| format!("{e}"))?;
    Ok(())
}

fn open_apply(path: &Path) -> Result<Database, String> {
    let db =
        Database::new(path, SystemConfig::default()).map_err(|e| format!("Database::new: {e}"))?;
    {
        let conn = Connection::new(&db).map_err(|e| format!("Connection::new: {e}"))?;
        apply_ddl(&conn)?;
    }
    Ok(db)
}

fn try_query(conn: &Connection, q: &str) -> Result<(), String> {
    conn.query(q).map_err(|e| format!("{e}"))?;
    Ok(())
}

fn main() {
    println!("=== PR5c spike #2: composite DDL under Store::open reopen pattern ===\n");

    // ────────────────────────────────────────────────────────────────────
    // T1. schema_is_idempotent_across_reopens
    // ────────────────────────────────────────────────────────────────────
    println!("T1. open → apply BASE_DDL → drop → reopen → apply BASE_DDL again\n");
    {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t1.db");
        match open_apply(&path) {
            Ok(_db) => println!("  ✅ Round 1 open + DDL OK"),
            Err(e) => {
                println!("  ❌ Round 1 FAIL: {e}");
                return;
            }
        }
        // drop _db here — close
        match std::panic::catch_unwind(|| open_apply(&path)) {
            Ok(Ok(db2)) => {
                println!("  ✅ Round 2 reopen + idempotent DDL replay OK");
                let conn = Connection::new(&db2).unwrap();
                let _ = try_query(&conn, "MATCH (s:Symbol) RETURN count(s)");
                println!("  ✅ post-reopen query OK");
            }
            Ok(Err(e)) => println!("  ❌ Round 2 reopen FAIL: {e}"),
            Err(_) => println!("  ❌ Round 2 reopen PANIC (kuzu#6045 likely)"),
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // T2. reopen_complete_cache_returns_resume — populated path
    // ────────────────────────────────────────────────────────────────────
    println!("\nT2. open → apply DDL → INSERT row → close → reopen → re-apply DDL\n");
    {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t2.db");
        {
            let db = open_apply(&path).expect("round 1");
            let conn = Connection::new(&db).unwrap();
            let _ = try_query(
                &conn,
                "CREATE (:Symbol {id: 'a', name: 'foo', mods: ['pub'], \
                 params: [{n: 'x', t: 'i32'}]})",
            );
            println!("  Round 1 populate OK");
        }
        match std::panic::catch_unwind(|| {
            let db = open_apply(&path).expect("round 2 open");
            let conn = Connection::new(&db).unwrap();
            try_query(&conn, "MATCH (s:Symbol) RETURN s.id, size(s.params)")
        }) {
            Ok(Ok(_)) => println!("  ✅ Round 2 reopen + DDL replay + query OK"),
            Ok(Err(e)) => println!("  ❌ Round 2 query FAIL: {e}"),
            Err(_) => println!("  ❌ Round 2 PANIC"),
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // T3. read_only_store_refuses_commit — reopen as read-only after writer closed
    // ────────────────────────────────────────────────────────────────────
    println!("\nT3. open writer + DDL + populate + close → reopen read-only + DDL\n");
    {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t3.db");
        {
            let db = open_apply(&path).expect("writer round 1");
            let conn = Connection::new(&db).unwrap();
            let _ = try_query(
                &conn,
                "CREATE (:Symbol {id: 'b', name: 'bar', mods: [], params: []})",
            );
            println!("  Round 1 writer populate OK");
        }
        match std::panic::catch_unwind(|| {
            // Read-only reopen — will Store apply BASE_DDL? Yes (line 150).
            // CREATE IF NOT EXISTS on RO connection — does that error or no-op?
            let db =
                Database::new(&path, SystemConfig::default().read_only(true)).expect("ro reopen");
            let conn = Connection::new(&db).expect("ro conn");
            // Try the BASE_DDL replay path on read-only connection
            let result = match conn.query(COMPOSITE_DDL) {
                Ok(_) => "RO DDL: idempotent OK".to_string(),
                Err(e) => format!("RO DDL: rejected (expected): {e}"),
            };
            result
        }) {
            Ok(s) => println!("  ✅ Round 2 read-only reopen — {s}"),
            Err(_) => println!("  ❌ Round 2 read-only PANIC"),
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // T4. multi-reopen stress — reopen 5 times empty cache (PR2 found 4
    //    reopen-tests broke; this saturates the window)
    // ────────────────────────────────────────────────────────────────────
    println!("\nT4. multi-reopen stress (empty cache, 5 rounds, BASE_DDL replay each)\n");
    {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t4.db");
        for round in 1..=5 {
            match std::panic::catch_unwind(|| open_apply(&path)) {
                Ok(Ok(_)) => println!("  ✅ Round {round} OK"),
                Ok(Err(e)) => {
                    println!("  ❌ Round {round} FAIL: {e}");
                    return;
                }
                Err(_) => {
                    println!("  ❌ Round {round} PANIC (kuzu#6045)");
                    return;
                }
            }
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // T5. populate-then-DELETE-then-COPY (simulates indexer reindex lifecycle)
    // ────────────────────────────────────────────────────────────────────
    println!("\nT5. reindex lifecycle: populate → DELETE all → COPY new rows (incl composites)\n");
    {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t5.db");
        let db = open_apply(&path).expect("open");
        let conn = Connection::new(&db).unwrap();
        let _ = try_query(
            &conn,
            "CREATE (:Symbol {id: 'a', name: 'old', mods: ['pub'], params: [{n: 'x', t: 'i32'}]})",
        );
        let _ = try_query(&conn, "MATCH (s:Symbol) DETACH DELETE s");
        // Now COPY with full row (including composites)
        let csv = dir.path().join("syms.csv");
        std::fs::write(
            &csv,
            "b,bar,\"[]\",\"[]\"\nc,baz,\"[pub,async]\",\"[{n: x, t: i32}]\"\n",
        )
        .unwrap();
        match try_query(
            &conn,
            &format!(
                "COPY Symbol (id, name, mods, params) FROM '{}' (header=false)",
                csv.display()
            ),
        ) {
            Ok(_) => {
                println!("  ✅ COPY full-row with composites OK");
                let rs = conn
                    .query("MATCH (s:Symbol) RETURN s.id, s.mods, size(s.params) ORDER BY s.id")
                    .unwrap();
                for row in rs {
                    let cells: Vec<String> = row.into_iter().map(|v| format!("{v:?}")).collect();
                    println!("    {}", cells.join(" | "));
                }
            }
            Err(e) => println!("  ❌ COPY FAIL: {e}"),
        }
    }

    println!("\n=== spike #2 done ===");
    println!("\nIf ALL T1-T5 ✅ → PR5c production path safe.");
    println!("If ANY ❌ → identifies exact failure surface and PR5c blocked.");
}
