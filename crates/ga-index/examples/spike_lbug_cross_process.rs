//! v1.5 PR6.1 (multi-mcp) cross-process lbug coexistence spike.
//!
//! Scenario the user explicitly asked for:
//!   "2 MCP servers cùng repo đều chủ động reindex được qua tool
//!    ga_reindex (ở các thời điểm khác nhau, không đồng thời)."
//!
//! Under the PR6.1 design, the writer process releases its exclusive
//! flock at `seal_for_serving`. When a second `graphatlas mcp` boots
//! against the same repo, its `Store::open_with_root_and_schema` will
//! `try_acquire_exclusive` and SUCCEED — then attempt to open
//! `lbug::Database` in RW mode. The first MCP's process is still alive
//! with an RO `lbug::Database` handle on the same `graph.db` file.
//!
//! This spike answers: does lbug 0.16.1 allow cross-process RW + RO
//! coexistence on the same on-disk database file?
//!
//! Matrix exercised:
//!   1. parent RO + child RO     — expected OK (two readers)
//!   2. parent RO + child RW     — the realistic 2-MCP boot scenario
//!   3. parent RW + child RO     — inverse (uncommon but symmetric)
//!   4. parent RW + child RW     — should be refused (two writers)
//!
//! Source DB resolution: same as `spike_no_reader_flock_real_fixture.rs`
//! — looks for `~/.graphatlas/{django,gin,agentfolk-frontend}-*/graph.db`
//! or honors `GA_SPIKE_SOURCE_DB`.
//!
//! Run:
//!   LBUG_BUILD_FROM_SOURCE=1 cargo run --release -p ga-index \
//!     --example spike_lbug_cross_process

use lbug::{Connection, Database, SystemConfig};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

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
    Err("No real fixture graph.db found".to_string())
}

fn helper_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join("../../target/release/ga_index_lbug_opener"),
        manifest_dir.join("../../target/debug/ga_index_lbug_opener"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.canonicalize().expect("canonicalize");
        }
    }
    panic!(
        "ga_index_lbug_opener not found. Run: cargo build --release -p ga-index --bin ga_index_lbug_opener"
    );
}

fn open_parent(path: &Path, mode: &str) -> Result<Database, String> {
    let cfg = match mode {
        "ro" => SystemConfig::default().read_only(true),
        "rw" => SystemConfig::default(),
        _ => unreachable!(),
    };
    Database::new(path, cfg).map_err(|e| format!("parent open {mode}: {e}"))
}

fn parent_query(db: &Database) -> Result<i64, String> {
    let conn = Connection::new(db).map_err(|e| format!("conn: {e}"))?;
    let rs = conn
        .query("MATCH (s:Symbol) RETURN count(s)")
        .map_err(|e| format!("query: {e}"))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n);
        }
    }
    Ok(-1)
}

/// Spawn the helper with the given mode against `path`. Returns (status,
/// first_line_of_stdout). The helper prints `OK <n>` on success or
/// `ERR <msg>` on failure.
fn spawn_child(path: &Path, mode: &str, secs: u64) -> (bool, String) {
    let bin = helper_path();
    let mut child = Command::new(&bin)
        .arg("--path")
        .arg(path)
        .arg("--mode")
        .arg(mode)
        .arg("--secs")
        .arg(secs.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn child: {e}"));
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).ok();
    let ok = line.starts_with("OK");
    let status = child.wait().expect("wait child");
    let combined = if !ok && !status.success() {
        format!("{} (exit {})", line.trim(), status.code().unwrap_or(-1))
    } else {
        line.trim().to_string()
    };
    (ok, combined)
}

/// Hold the parent handle in a thread while a child runs concurrently;
/// the child holds its own handle for `--secs`. Both ALIVE at the same
/// instant — this is the load-bearing check (not "open serially").
fn race_test(label: &str, target_db: &Path, parent_mode: &str, child_mode: &str) {
    println!("\n--- {label}: parent={parent_mode} + child={child_mode} (concurrent) ---");

    // Parent opens first.
    let parent_db = match open_parent(target_db, parent_mode) {
        Ok(d) => d,
        Err(e) => {
            println!("  FAIL parent open: {e}");
            return;
        }
    };
    let parent_count = parent_query(&parent_db);
    println!("  parent open OK; query: {:?}", parent_count);

    // Child runs concurrently.
    let (child_ok, child_line) = spawn_child(target_db, child_mode, 2);
    if child_ok {
        println!("  child {child_mode} open OK: {child_line}");
    } else {
        println!("  child {child_mode} open FAIL: {child_line}");
    }

    // After child returns, query parent again to verify the parent's handle
    // didn't get corrupted by the child's lifecycle.
    let post = parent_query(&parent_db);
    match (&parent_count, &post) {
        (Ok(a), Ok(b)) if a == b => {
            println!("  parent post-query OK: count unchanged ({a})")
        }
        (Ok(a), Ok(b)) => {
            println!("  parent post-query DRIFT: before={a} after={b}")
        }
        (_, Err(e)) => println!("  parent post-query FAIL: {e}"),
        _ => {}
    }
    drop(parent_db);
}

fn main() {
    println!("=== v1.5 PR6.1 cross-process lbug coexistence spike ===");

    let source_db = locate_source_db().unwrap_or_else(|e| panic!("{e}"));
    let mb = std::fs::metadata(&source_db).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
    println!("\n[setup] source DB: {} ({mb:.2} MB)", source_db.display());

    let dir = TempDir::new().unwrap();
    let target_db = dir.path().join("graph.db");
    std::fs::copy(&source_db, &target_db).expect("copy");
    println!("[setup] working copy at {}", target_db.display());

    // Wait a moment so the helper binary's stderr from a prior spike doesn't
    // intermix with this spike's stdout.
    std::thread::sleep(Duration::from_millis(100));

    race_test("Q1 RO/RO", &target_db, "ro", "ro");
    race_test("Q2 RO/RW (2-MCP boot scenario)", &target_db, "ro", "rw");
    race_test("Q3 RW/RO (inverse)", &target_db, "rw", "ro");
    race_test("Q4 RW/RW (two writers)", &target_db, "rw", "rw");

    println!("\n=== SUMMARY ===");
    println!("If Q2 (RO/RW) succeeds with no parent drift: 2-MCP boot works as designed.");
    println!("If Q2 fails: Store::open needs to detect 'cache complete + lbug RW blocked' and fall through to open_read_only.");
}
