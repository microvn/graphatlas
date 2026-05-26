//! Cross-process lbug coexistence helper — v1.5 PR6.1 (multi-mcp)
//! post-build verification.
//!
//! Takes `--path <graph.db> --mode {ro|rw} --secs <N>`. Opens
//! `lbug::Database` in the requested mode, runs one trivial query to
//! prove the handle is usable, then prints `OK <node_count>` to stdout
//! and holds the handle for `--secs` seconds (so a sibling process can
//! attempt the other-mode open against the same file). Prints
//! `ERR <message>` on any failure and exits non-zero.

use lbug::{Connection, Database, SystemConfig};
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path: Option<PathBuf> = None;
    let mut mode = "ro".to_string();
    let mut secs: u64 = 5;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--path" => {
                i += 1;
                path = Some(PathBuf::from(&args[i]));
            }
            "--mode" => {
                i += 1;
                mode = args[i].clone();
            }
            "--secs" => {
                i += 1;
                secs = args[i].parse().expect("--secs u64");
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let path = path.expect("--path required");

    let cfg = match mode.as_str() {
        "ro" => SystemConfig::default().read_only(true),
        "rw" => SystemConfig::default(),
        other => {
            println!("ERR unknown --mode {other}");
            std::process::exit(2);
        }
    };

    let db = match Database::new(&path, cfg) {
        Ok(d) => d,
        Err(e) => {
            println!("ERR open {mode} {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    let conn = match Connection::new(&db) {
        Ok(c) => c,
        Err(e) => {
            println!("ERR connection {mode}: {e}");
            std::process::exit(1);
        }
    };

    // Light query — count nodes if Symbol table exists; else just a constant.
    let count = match conn.query("MATCH (s:Symbol) RETURN count(s)") {
        Ok(rs) => {
            let mut n: i64 = -1;
            for row in rs {
                if let Some(lbug::Value::Int64(v)) = row.into_iter().next() {
                    n = v;
                }
            }
            n
        }
        Err(e) => {
            println!("ERR query {mode}: {e}");
            std::process::exit(1);
        }
    };

    println!("OK {count}");

    // Hold the handle so the sibling process can race against us.
    std::thread::sleep(Duration::from_secs(secs));
    drop(conn);
    drop(db);
}
