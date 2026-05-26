//! v1.5 PR6.1 (reindex-multi-mcp) spike — verify the load-bearing
//! assumptions of the "reader does not hold a long-lived shared flock"
//! design.
//!
//! Run:
//!   LBUG_BUILD_FROM_SOURCE=1 cargo run -p ga-index --example spike_no_reader_flock
//!
//! Questions we answer here:
//!
//!   Q1. Can `lbug::Database::new(path, read_only(true))` open a committed
//!       graph.db WITHOUT any flock held at our (process) layer? (Our flock
//!       is application-level — kernel POSIX flock — not enforced by lbug.)
//!
//!   Q2. While a RO handle is open, an external actor (simulating the writer
//!       in another process) `unlink`s the underlying graph.db. Does the
//!       RO handle continue to serve queries from the still-alive inode?
//!       (POSIX promise: yes, until last fd closes.)
//!
//!   Q3. After the unlink, the external actor creates a fresh empty lbug DB
//!       at the same path. Does the RO handle's reads now see (a) old data
//!       — confirming POSIX inode persistence, (b) new empty data, or
//!       (c) crash / SIGBUS / corruption?
//!
//! If Q1+Q2 PASS and Q3 returns OLD data (or any non-crash behavior), the
//! design works: readers stay alive while a writer in another process
//! reindexes; new clients reopen the new file and see fresh data.
//!
//! Q3 returning NEW data would also be acceptable for our use case (no
//! corruption, just a generation skip) but would mean we can't rely on
//! point-in-time read snapshots across a rebuild.
//!
//! Q3 crashing means lbug uses mmap aggressively and we MUST take a
//! short per-tool-call shared flock to gate the writer's nuke. Fall back
//! plan documented in the parent spec.

use lbug::{Connection, Database, SystemConfig};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn create_db_with_row(path: &Path, row_name: &str) -> Result<(), String> {
    let db = Database::new(path, SystemConfig::default())
        .map_err(|e| format!("Database::new RW: {e}"))?;
    let conn = Connection::new(&db).map_err(|e| format!("Connection::new: {e}"))?;
    conn.query("CREATE NODE TABLE IF NOT EXISTS T(id STRING PRIMARY KEY, name STRING)")
        .map_err(|e| format!("DDL: {e}"))?;
    conn.query(&format!("CREATE (:T {{id: 'r1', name: '{row_name}'}})"))
        .map_err(|e| format!("INSERT: {e}"))?;
    Ok(())
}

fn open_ro(path: &Path) -> Result<Database, String> {
    Database::new(path, SystemConfig::default().read_only(true))
        .map_err(|e| format!("Database::new RO: {e}"))
}

fn read_row_name(db: &Database) -> Result<Vec<String>, String> {
    let conn = Connection::new(db).map_err(|e| format!("Connection: {e}"))?;
    let rs = conn
        .query("MATCH (t:T) RETURN t.name")
        .map_err(|e| format!("query: {e}"))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.push(s);
        }
    }
    Ok(out)
}

fn nuke_path(path: &Path) {
    // Mirrors what reindex_in_place does: removes the on-disk file.
    let _ = std::fs::remove_file(path);
    // lbug may have written sidecar files (.wal etc); best-effort drop.
    if let Some(parent) = path.parent() {
        for ext in ["wal", "shm", "lock"] {
            let sib = parent.join(format!(
                "{}.{ext}",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("graph.db")
            ));
            let _ = std::fs::remove_file(&sib);
        }
    }
}

fn run() -> Result<(), String> {
    println!("=== v1.5 reindex-multi-mcp spike: reader without long-lived flock ===\n");

    let dir = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let db_path: PathBuf = dir.path().join("graph.db");

    // Seed: committed DB with row1.
    println!("[setup] create initial DB at {}", db_path.display());
    create_db_with_row(&db_path, "row1")?;
    println!("[setup] OK");

    // ────────────────────────────────────────────────────────────────────
    // Q1 — open RO without any flock at our layer
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Q1: open lbug RO without any application-level flock ---");
    let reader = match open_ro(&db_path) {
        Ok(db) => {
            println!("  PASS  lbug RO open succeeded (no flock needed at lbug layer)");
            db
        }
        Err(e) => {
            println!("  FAIL  lbug RO open errored: {e}");
            return Err("Q1 FAILED — fallback to per-call shared flock required".into());
        }
    };

    match read_row_name(&reader) {
        Ok(rows) => println!("  PASS  initial RO query: {:?}", rows),
        Err(e) => {
            println!("  FAIL  initial RO query: {e}");
            return Err("Q1 FAILED — RO handle not usable".into());
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Q2 — external unlink while RO handle is alive
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Q2: external nuke while RO handle still open ---");
    nuke_path(&db_path);
    println!("  setup: graph.db unlinked (file missing on disk)");
    assert!(!db_path.exists(), "expected on-disk file to be unlinked");

    let q2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_row_name(&reader)));
    match q2 {
        Ok(Ok(rows)) => {
            println!(
                "  PASS  RO query after unlink still works (rows: {:?}); \
                 POSIX inode persistence confirmed",
                rows
            );
        }
        Ok(Err(e)) => {
            println!("  FAIL  RO query after unlink errored: {e}");
            return Err("Q2 FAILED — reader does not survive file deletion".into());
        }
        Err(_) => {
            println!("  FAIL  RO query after unlink PANICKED (mmap fault likely)");
            return Err("Q2 FAILED — reader crashed under writer nuke".into());
        }
    }

    // ────────────────────────────────────────────────────────────────────
    // Q3 — external recreate fresh DB at same path; existing reader keeps querying
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Q3: external recreates a fresh DB at the same path ---");
    create_db_with_row(&db_path, "row2_new_generation")?;
    println!("  setup: new graph.db created at same path with different row");

    let q3 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_row_name(&reader)));
    match q3 {
        Ok(Ok(rows)) => {
            if rows.iter().any(|s| s == "row1") {
                println!(
                    "  PASS  existing RO reader still sees OLD inode data ({:?}) — \
                     point-in-time snapshot preserved",
                    rows
                );
            } else {
                println!(
                    "  SOFT-PASS  existing RO reader sees NEW data ({:?}) — \
                     no crash, but snapshot semantics not preserved across nuke",
                    rows
                );
            }
        }
        Ok(Err(e)) => {
            println!(
                "  SOFT-FAIL  RO query after recreate errored: {e} — \
                 reader handle invalidated but no crash; consider \
                 reopen_if_stale pattern in tools layer"
            );
        }
        Err(_) => {
            println!("  HARD-FAIL  RO query after recreate PANICKED");
            return Err("Q3 HARD-FAIL — reader crashes during writer rebuild; \
                        fall back to per-tool-call shared flock"
                .into());
        }
    }

    // Bonus: a brand-new reader opening the new path sees new data.
    println!("\n--- Bonus: brand-new reader after Q3 sees fresh-generation data ---");
    match open_ro(&db_path).and_then(|db| read_row_name(&db)) {
        Ok(rows) => println!("  PASS  fresh reader sees: {:?}", rows),
        Err(e) => println!("  FAIL  fresh reader: {e}"),
    }

    println!("\n=== SPIKE SUMMARY ===");
    println!("Q1 (RO open w/o flock): PASS");
    println!("Q2 (RO survives unlink): see above");
    println!("Q3 (RO during recreate): see above");
    println!(
        "\nDecision:\n  - Q1 PASS + Q2 PASS + Q3 PASS/SOFT-PASS => proceed with reader-no-flock design.\n  - Any HARD-FAIL => fall back to per-tool-call short shared flock."
    );
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nSPIKE OUTCOME: {e}");
        std::process::exit(1);
    }
}
