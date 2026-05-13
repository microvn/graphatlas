//! Manual review export — 3 axum tasks chosen to span the 3 failure modes:
//!
//! 1. `fmt` (size=3) — generic seed; tests Fix #1 (seed_file_hint)
//! 2. `FromRequestParts` (size=8, multi-crate) — tests workspace dispersion
//! 3. `infer_state_type_from_field_attributes` (size=9, 3 examples/) — tests
//!     examples/ inclusion + cross-crate dispersion combined
//!
//! Run:
//!   cargo test -p ga-query --test _review_axum_3tasks -- --ignored --nocapture
//!
//! Hits use the SAME engine call shape as the M3 bench
//! (for_symbol_in_file with seed_file hint).

use ga_index::Store;
use std::path::PathBuf;
use std::process::Command;

fn git_checkout(repo: &std::path::Path, sha: &str) {
    let s = Command::new("git")
        .args(["-C", &repo.display().to_string(), "checkout", "-q", sha])
        .status()
        .expect("git checkout");
    assert!(s.success(), "checkout {sha} failed");
}

#[test]
#[ignore]
fn review_axum_3tasks() {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let fixture = repo_root.join("benches/fixtures/axum");

    // Per-task pinning: matches what M3 bench does. WITHOUT pinning, the
    // engine sees fixture HEAD which has files reorganised since the fix
    // commit (e.g. from_request.rs → from_request/mod.rs), so the seed
    // hint points at a stale path and the engine falls back wrong.
    let tasks: Vec<(&str, &str, &str, &str, Vec<&str>)> = vec![
        (
            "axum-7cbacd14",
            "ee0b71a4accfc42929fdfaf1664c0fb96b62b24a",
            "fmt",
            "axum-macros/src/from_request.rs",
            vec![
                "axum-macros/src/debug_handler.rs",
                "axum-macros/src/from_request.rs",
                "axum-macros/src/lib.rs",
            ],
        ),
        (
            "axum-d5de3bc7",
            "2e3000f1a302b04f112d78aba4ef591fdcb2dc54",
            "FromRequestParts",
            "axum-core/src/extract/mod.rs",
            vec![
                "axum-core/build.rs",
                "axum-core/src/extract/mod.rs",
                "axum-core/src/lib.rs",
                "axum-macros/src/debug_handler.rs",
                "axum-macros/src/lib.rs",
                "axum/build.rs",
                "axum/src/handler/mod.rs",
                "axum/src/lib.rs",
            ],
        ),
        (
            "axum-934b1aac",
            "94901e0fe798d0fb38b9e6d122376c15264ca46a",
            "infer_state_type_from_field_attributes",
            "axum-macros/src/from_request.rs",
            vec![
                "axum-core/src/response/into_response.rs",
                "axum-core/src/response/into_response_parts.rs",
                "axum-extra/src/json_lines.rs",
                "axum-extra/src/routing/typed.rs",
                "axum-macros/src/from_request.rs",
                "axum/benches/benches.rs",
                "examples/low-level-openssl/src/main.rs",
                "examples/low-level-rustls/src/main.rs",
                "examples/serve-with-hyper/src/main.rs",
            ],
        ),
    ];

    // Save HEAD so we can restore.
    let original_head = String::from_utf8(
        Command::new("git")
            .args(["-C", &fixture.display().to_string(), "rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    println!();
    for (task_id, base_commit, sym, seed_file, expected) in &tasks {
        println!("════════════════════════════════════════════════════════════════════");
        println!("TASK     {}", task_id);
        println!("SYMBOL   {}", sym);
        println!("SEED_FILE {}", seed_file);
        println!("PIN      {}", base_commit);
        println!("────────────────────────────────────────────────────────────────────");

        // Pin per task (matches M3 bench).
        git_checkout(&fixture, base_commit);

        // Fresh store + index per task (avoids stale cache from previous pin).
        let cache = repo_root
            .join(".graphatlas-bench-cache/m3-minimal_context/axum/review")
            .join(*task_id);
        let _ = std::fs::remove_dir_all(&cache);
        let store = Store::open_with_root(&cache, &fixture).expect("open store");
        ga_query::indexer::build_index(&store, &fixture).expect("build_index");

        let req = ga_query::minimal_context::MinimalContextRequest::for_symbol_in_file(
            *sym, *seed_file, 2000,
        );
        let resp = match ga_query::minimal_context::minimal_context(&store, &req) {
            Ok(r) => r,
            Err(e) => {
                println!("  ERROR: {e}");
                println!();
                continue;
            }
        };
        let actual_files: std::collections::BTreeSet<&str> =
            resp.symbols.iter().map(|s| s.file.as_str()).collect();

        println!("EXPECTED ({} files):", expected.len());
        for f in expected.iter() {
            let hit = actual_files.contains(f);
            let mark = if hit { "  HIT  " } else { "  MISS " };
            println!("{}{}", mark, f);
        }
        let hits = expected
            .iter()
            .filter(|f| actual_files.contains(*f))
            .count();
        let recall = hits as f64 / expected.len() as f64;
        println!("recall = {hits}/{} = {:.3}", expected.len(), recall);

        println!("────────────────────────────────────────────────────────────────────");
        println!("ENGINE RESPONSE ({} contexts):", resp.symbols.len());
        for ctx in &resp.symbols {
            let in_gt = expected.iter().any(|e| *e == ctx.file);
            let mark = if in_gt { "  ✓  " } else { "  ·  " };
            println!(
                "{}[{:?}] {} ({} tokens)  {}",
                mark, ctx.reason, ctx.symbol, ctx.tokens, ctx.file
            );
        }
        println!(
            "  budget_used = {:.2}, token_estimate = {}, truncated = {}",
            resp.budget_used, resp.token_estimate, resp.meta.truncated
        );
        println!();
    }
    println!("════════════════════════════════════════════════════════════════════");
    // Restore HEAD.
    git_checkout(&fixture, &original_head);
}
