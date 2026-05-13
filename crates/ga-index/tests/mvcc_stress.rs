//! S-003 AS-009 data-line: short-run MVCC stress (1 writer + 10 readers).
//!
//! This is the in-suite equivalent of the 10-minute 20-reader spike run
//! captured in docs/adr/001-graph-storage.md. Here we cap at ~2 seconds so
//! `cargo test` stays fast; the spike is the authoritative correctness proof.

use ga_index::Store;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn one_writer_ten_readers_no_torn_state_over_2s() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/mvcc-stress");

    // Open store + create canary table.
    let store = Arc::new(Store::open_with_root(&cache_root, repo).unwrap());
    {
        let c = store.connection().unwrap();
        let _ = c.query(
            "CREATE NODE TABLE IF NOT EXISTS K(id STRING, batch INT64, v INT64, PRIMARY KEY(id))",
        );
    }

    let stop = Arc::new(AtomicBool::new(false));
    let torn = Arc::new(AtomicU64::new(0));
    let reads = Arc::new(AtomicU64::new(0));
    let writes = Arc::new(AtomicU64::new(0));

    const BATCH_N: i64 = 25; // smaller than the spike's 200 so 2s is enough.

    // Writer thread.
    let writer = {
        let store = store.clone();
        let stop = stop.clone();
        let writes = writes.clone();
        thread::spawn(move || {
            let mut batch_id = 0i64;
            while !stop.load(Ordering::Relaxed) {
                let c = store.connection().unwrap();
                let mut parts = Vec::with_capacity(BATCH_N as usize);
                for i in 0..BATCH_N {
                    parts.push(format!(
                        "(:K {{id: 'b{batch_id}-{i}', batch: {batch_id}, v: {i}}})"
                    ));
                }
                let q = format!("CREATE {}", parts.join(","));
                if c.query(&q).is_ok() {
                    writes.fetch_add(1, Ordering::Relaxed);
                }
                batch_id += 1;
                thread::sleep(Duration::from_millis(5));
            }
        })
    };

    // 10 reader threads.
    let readers: Vec<_> = (0..10)
        .map(|_rid| {
            let store = store.clone();
            let stop = stop.clone();
            let torn = torn.clone();
            let reads = reads.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let c = store.connection().unwrap();
                    if let Ok(rs) = c.query("MATCH (k:K) RETURN k.batch") {
                        let mut counts: std::collections::HashMap<i64, u32> =
                            std::collections::HashMap::new();
                        for row in rs {
                            if let Some(lbug::Value::Int64(b)) = row.into_iter().next() {
                                *counts.entry(b).or_insert(0) += 1;
                            }
                        }
                        reads.fetch_add(1, Ordering::Relaxed);
                        // Any per-batch count that is neither 0 nor BATCH_N is torn.
                        for n in counts.values() {
                            if *n != BATCH_N as u32 {
                                torn.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                }
            })
        })
        .collect();

    thread::sleep(Duration::from_secs(2));
    stop.store(true, Ordering::Relaxed);

    writer.join().unwrap();
    for r in readers {
        r.join().unwrap();
    }

    let n_reads = reads.load(Ordering::Relaxed);
    let n_writes = writes.load(Ordering::Relaxed);
    let n_torn = torn.load(Ordering::Relaxed);

    eprintln!("mvcc_stress: reads={n_reads} writes={n_writes} torn={n_torn}");
    assert!(n_reads >= 10, "readers did not run at all: {n_reads}");
    assert!(n_writes >= 5, "writer did not run enough: {n_writes}");
    assert_eq!(n_torn, 0, "MVCC torn state observed {n_torn} times");
}
