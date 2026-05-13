//! EXP-M2-01 — BFS global visited-node cap (port from rust-poc).
//!
//! Hub symbols with thousands of incoming edges explode BFS visited-set
//! and blow the query latency budget. Rust-poc caps at `MAX_IMPACT_NODES =
//! 500` (`rust-poc/src/main.rs:2152`); this test validates the cap bites
//! when the visited set would otherwise exceed 500.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn bfs_visited_cap_bounds_impacted_files_on_hub_seed() {
    // 550 distinct caller files, each defines one function that calls `seed`.
    // Without cap, BFS visits all 550 symbols + seed = 551 visited — output
    // file list grows with it (551 files, one seed + 550 callers).
    // With MAX_VISITED=500, BFS stops early — output is strictly bounded.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(&repo.join("seed.py"), "def seed():\n    pass\n");
    for i in 0..550 {
        write(
            &repo.join(format!("c{i:04}.py")),
            &format!("from seed import seed\ndef f{i}():\n    seed()\n"),
        );
    }

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("seed".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // The BFS visited-set cap is 500 symbols; the seed is inserted before
    // the loop, so at most 500 caller symbols get expanded. Seed file +
    // caller files surfaced: should be strictly below 551.
    //
    // Upper bound check is the gate: without cap, len() == 551. With cap,
    // len() <= 501 (seed + up to 500 callers hit before break). Allow slack
    // to 510 to cover rust-poc's cap placement (check at start of depth
    // loop — may admit up to max_visited + chunk_size symbols).
    assert!(
        resp.impacted_files.len() <= 510,
        "BFS visited cap must bound impacted_files to ~500 on hub seed; got {} (cap ineffective — 551 = uncapped)",
        resp.impacted_files.len(),
    );

    // total_available still reports pre-cap count so callers can tell the
    // cap bit. This check also protects against future refactors that
    // accidentally bypass the cap.
    // Note: Tools-C5 apply_output_caps also truncates at OUTPUT_CAP=50,
    // but `total_available` records pre-OUTPUT_CAP counts. If BFS visits
    // 551 pre-cap, total_available would be 551; if BFS caps at 500,
    // total_available is ~501.
    assert!(
        resp.meta.total_available.impacted_files <= 510,
        "total_available.impacted_files should reflect BFS cap bite; got {}",
        resp.meta.total_available.impacted_files,
    );
}
