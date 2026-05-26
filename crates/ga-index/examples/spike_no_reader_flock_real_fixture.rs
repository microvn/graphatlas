//! v1.5 PR6.1 (reindex-multi-mcp) — C-4 follow-up spike on a REAL fixture.
//!
//! The original spike (`spike_no_reader_flock.rs`) ran with a 12-byte
//! 1-row DB and PASSed Q1/Q2/Q3 — but /mf-challenge C-4 pointed out that
//! lbug 0.16.1 likely mmaps page regions for non-trivial graphs, and a
//! tiny in-cache DB never exercises that path. SIGBUS on mmap'd file
//! truncation is a documented hazard in SQLite-class engines.
//!
//! This spike repeats Q1/Q2/Q3 with a REAL ~30 MB graph.db (django
//! fixture's existing cache). Reader runs `MATCH` queries in a hot loop
//! while a background thread nukes + recreates the file from a backup
//! copy 5 times. Pass criteria: no SIGBUS, no panic, no Err from
//! `connection()`/`query()`; query latency tracked for sanity.
//!
//! Setup: the spike needs `~/.graphatlas/django-2d483d/graph.db` (or any
//! other ~10MB+ ga-built graph.db) as input. We DO NOT rebuild from
//! scratch (would take ~30s of index time and adds nothing to the
//! load-bearing question).
//!
//! Run:
//!   LBUG_BUILD_FROM_SOURCE=1 cargo run --release -p ga-index \
//!     --example spike_no_reader_flock_real_fixture
//!
//! Or override the source DB:
//!   GA_SPIKE_SOURCE_DB=/path/to/graph.db cargo run --release ...

use lbug::{Connection, Database, SystemConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const HOT_LOOP_QUERIES: &[&str] = &[
    "MATCH (s:Symbol) RETURN count(s)",
    "MATCH (f:File) RETURN count(f)",
    "MATCH (s:Symbol)-[r:CALLS]->(t:Symbol) RETURN count(r)",
];

fn locate_source_db() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("GA_SPIKE_SOURCE_DB") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!(
            "GA_SPIKE_SOURCE_DB does not exist: {}",
            pb.display()
        ));
    }
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let candidates = ["django-2d483d", "gin-3c3fba", "agentfolk-frontend-fd0df4"];
    for c in &candidates {
        let pb = PathBuf::from(&home)
            .join(".graphatlas")
            .join(c)
            .join("graph.db");
        if pb.exists() {
            return Ok(pb);
        }
    }
    Err(format!(
        "No real fixture graph.db found at ~/.graphatlas/{{django,gin,agentfolk-frontend}}-*. \
         Build a fixture first (`graphatlas index <fixture>`) or set GA_SPIKE_SOURCE_DB."
    ))
}

fn open_ro(path: &Path) -> Result<Database, String> {
    Database::new(path, SystemConfig::default().read_only(true))
        .map_err(|e| format!("Database::new RO: {e}"))
}

fn run_one_query(db: &Database, cypher: &str) -> Result<i64, String> {
    let conn = Connection::new(db).map_err(|e| format!("Connection: {e}"))?;
    let rs = conn
        .query(cypher)
        .map_err(|e| format!("query [{cypher}]: {e}"))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n);
        }
    }
    Ok(0)
}

struct WriterStats {
    cycles_done: AtomicU64,
    last_error: std::sync::Mutex<Option<String>>,
}

fn writer_loop(
    backup_path: PathBuf,
    target_path: PathBuf,
    stop: Arc<AtomicBool>,
    stats: Arc<WriterStats>,
    cycles: u64,
) {
    // Tiny initial delay so reader gets a few queries in first.
    std::thread::sleep(Duration::from_millis(500));
    for cycle in 0..cycles {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        // Mirror what reindex_in_place would do: nuke then recreate the
        // graph.db file. We skip the real lbug rebuild and just file-copy
        // the backup — the load-bearing question is "does the reader's
        // mmap survive file replacement?", not "does ga-query rebuild?".
        if let Err(e) = std::fs::remove_file(&target_path) {
            *stats.last_error.lock().unwrap() = Some(format!("cycle {cycle} remove_file: {e}"));
            break;
        }
        // Sleep briefly with file UNLINKED. Reader queries during this
        // window must serve from the still-open old inode.
        std::thread::sleep(Duration::from_millis(200));

        if let Err(e) = std::fs::copy(&backup_path, &target_path) {
            *stats.last_error.lock().unwrap() = Some(format!("cycle {cycle} copy: {e}"));
            break;
        }
        stats.cycles_done.fetch_add(1, Ordering::SeqCst);
        // Hold steady for a moment so reader can query the new inode
        // (via reopen, separately) — but the long-lived reader continues
        // on the old inode.
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn run() -> Result<(), String> {
    println!(
        "=== v1.5 PR6.1 C-4 real-fixture spike: \
         reader survives mmap'd file nuke+recreate ==="
    );

    let source_db = locate_source_db()?;
    let source_size = std::fs::metadata(&source_db).map(|m| m.len()).unwrap_or(0);
    println!(
        "\n[setup] source DB: {} ({:.2} MB)",
        source_db.display(),
        source_size as f64 / 1_048_576.0
    );
    if source_size < 1_048_576 {
        eprintln!(
            "[warn] source DB is < 1 MB; spike value reduced. \
             Real production caches are 10-100MB."
        );
    }

    let dir = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let target_db = dir.path().join("graph.db");
    let backup_db = dir.path().join("graph.db.backup");

    // Materialize: copy source → target + backup. Both writable so spike
    // can nuke target without touching the user's real cache.
    std::fs::copy(&source_db, &target_db).map_err(|e| format!("copy → target: {e}"))?;
    std::fs::copy(&source_db, &backup_db).map_err(|e| format!("copy → backup: {e}"))?;
    println!(
        "[setup] target+backup materialized in {}",
        dir.path().display()
    );

    // ────────────────────────────────────────────────────────────────────
    // Phase 1: open RO reader on real-size DB, verify queries work
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Phase 1: open RO + baseline queries ---");
    let reader = open_ro(&target_db)?;
    for cypher in HOT_LOOP_QUERIES {
        let t = Instant::now();
        let n = run_one_query(&reader, cypher)?;
        println!("  [{:>5} µs] {} = {}", t.elapsed().as_micros(), cypher, n);
    }
    println!("  PASS baseline queries");

    // ────────────────────────────────────────────────────────────────────
    // Phase 2: hot reader loop while writer nukes+recreates 5 times
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Phase 2: 5 cycles of nuke+recreate while reader queries hot ---");
    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(WriterStats {
        cycles_done: AtomicU64::new(0),
        last_error: std::sync::Mutex::new(None),
    });
    let cycles = 5;
    let writer_handle = {
        let backup = backup_db.clone();
        let target = target_db.clone();
        let stop = stop.clone();
        let stats = stats.clone();
        std::thread::spawn(move || writer_loop(backup, target, stop, stats, cycles))
    };

    let mut reader_query_count: u64 = 0;
    let mut reader_error_count: u64 = 0;
    let mut latencies_us: Vec<u128> = Vec::new();
    let mut last_error: Option<String> = None;
    let reader_start = Instant::now();
    let budget = Duration::from_secs(30);

    // Reader queries in a hot loop. Each query routes through the same
    // long-lived RO `Database` opened in Phase 1 — exactly what a peer
    // MCP process does between writer reindexes.
    while reader_start.elapsed() < budget && stats.cycles_done.load(Ordering::SeqCst) < cycles {
        let cypher = HOT_LOOP_QUERIES[reader_query_count as usize % HOT_LOOP_QUERIES.len()];
        let t = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_one_query(&reader, cypher)
        }));
        let elapsed = t.elapsed().as_micros();
        latencies_us.push(elapsed);
        reader_query_count += 1;
        match result {
            Ok(Ok(_n)) => {}
            Ok(Err(e)) => {
                reader_error_count += 1;
                if last_error.is_none() {
                    last_error = Some(e);
                }
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = writer_handle.join();
                return Err(format!(
                    "HARD-FAIL: reader query PANICKED on cycle {} (likely SIGBUS on mmap'd file truncation) \
                     after {} queries — fall back to per-tool-call short shared flock in PR6.1",
                    stats.cycles_done.load(Ordering::SeqCst),
                    reader_query_count
                ));
            }
        }
        // Tight loop, small sleep so we don't 100% pin CPU.
        std::thread::sleep(Duration::from_millis(5));
    }

    stop.store(true, Ordering::SeqCst);
    let _ = writer_handle.join();

    // ────────────────────────────────────────────────────────────────────
    // Phase 3: report
    // ────────────────────────────────────────────────────────────────────
    let cycles_done = stats.cycles_done.load(Ordering::SeqCst);
    println!("\n--- Phase 3: results ---");
    println!("  writer cycles completed:   {} / {}", cycles_done, cycles);
    if let Some(e) = stats.last_error.lock().unwrap().as_ref() {
        println!("  writer last error:         {}", e);
    }
    println!("  reader queries issued:     {}", reader_query_count);
    println!("  reader queries errored:    {}", reader_error_count);
    println!(
        "  reader error rate:         {:.2}%",
        100.0 * reader_error_count as f64 / reader_query_count.max(1) as f64
    );
    if let Some(e) = last_error {
        println!("  reader first error:        {}", e);
    }
    if !latencies_us.is_empty() {
        latencies_us.sort_unstable();
        let p50 = latencies_us[latencies_us.len() / 2];
        let p99 = latencies_us[(latencies_us.len() * 99) / 100];
        let max = *latencies_us.last().unwrap();
        println!("  reader latency µs:         p50={p50}  p99={p99}  max={max}");
    }

    // ────────────────────────────────────────────────────────────────────
    // Phase 4: post-rebuild, brand-new reader sees the same DB (via copy)
    // ────────────────────────────────────────────────────────────────────
    println!("\n--- Phase 4: brand-new reader after final cycle ---");
    match open_ro(&target_db).and_then(|db| run_one_query(&db, HOT_LOOP_QUERIES[0])) {
        Ok(n) => println!("  PASS fresh reader: {} = {}", HOT_LOOP_QUERIES[0], n),
        Err(e) => println!("  FAIL fresh reader: {e}"),
    }

    println!("\n=== SUMMARY ===");
    let summary = if reader_error_count == 0 && cycles_done == cycles {
        "HARD-PASS — no errors, no panics, all cycles completed"
    } else if reader_error_count < reader_query_count / 10 && cycles_done == cycles {
        "SOFT-PASS — some query errors (<10%), no panics; consider per-tool retry in handler"
    } else if cycles_done < cycles {
        "INCONCLUSIVE — writer side errored; rerun"
    } else {
        "SOFT-FAIL — high error rate but no panic; per-tool-call shared flock recommended"
    };
    println!("  {summary}");
    println!(
        "\nDecision:\n  HARD-PASS / SOFT-PASS => proceed with reader-no-flock design in PR6.1.\n  SOFT-FAIL or any PANIC => add per-tool-call shared flock fallback IN PR6.1 (not deferred)."
    );
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nSPIKE OUTCOME: {e}");
        std::process::exit(1);
    }
}
