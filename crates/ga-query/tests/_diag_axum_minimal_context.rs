//! Diagnostic — per-task minimal_context output for axum to understand
//! why M3 minimal_context FAIL on axum (file_recall 0.478 vs target 0.70).
//! Run:
//!   cargo test -p ga-query --test _diag_axum_minimal_context -- --ignored --nocapture
use ga_index::Store;
use std::path::PathBuf;

#[test]
#[ignore]
fn diag_axum_minimal_context() {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let cache = repo_root.join(".graphatlas-bench-cache/m3-minimal_context/axum/diag");
    let fixture = repo_root.join("benches/fixtures/axum");
    let _ = std::fs::remove_dir_all(&cache);
    let store = Store::open_with_root(&cache, &fixture).expect("open store");
    ga_query::indexer::build_index(&store, &fixture).expect("build_index");

    let tasks: Vec<(&str, Vec<&str>)> = vec![
        (
            "fmt",
            vec![
                "axum-macros/src/debug_handler.rs",
                "axum-macros/src/from_request.rs",
                "axum-macros/src/lib.rs",
            ],
        ),
        (
            "body",
            vec![
                "axum/src/extract/content_length_limit.rs",
                "axum/src/extract/mod.rs",
                "axum/src/extract/multipart.rs",
                "axum/src/extract/rejection.rs",
            ],
        ),
        (
            "PathRouter",
            vec!["axum/src/routing/mod.rs", "axum/src/routing/path_router.rs"],
        ),
        (
            "check_inputs_impls_from_request",
            vec!["axum-macros/src/debug_handler.rs"],
        ),
        ("expand_field", vec!["axum-macros/src/from_ref.rs"]),
        (
            "check_future_send",
            vec!["axum-macros/src/debug_handler.rs"],
        ),
        (
            "Router",
            vec![
                "axum/src/routing/method_routing.rs",
                "axum/src/routing/mod.rs",
                "axum/src/routing/path_router.rs",
            ],
        ),
        ("Route", vec!["axum/src/routing/route.rs"]),
        (
            "FromRequestParts",
            vec!["axum-core/src/extract/mod.rs", "axum-core/src/lib.rs"],
        ),
        (
            "strip_prefix",
            vec![
                "axum/src/extract/matched_path.rs",
                "axum/src/extract/path/mod.rs",
                "axum/src/routing/strip_prefix.rs",
            ],
        ),
    ];
    println!();
    for (sym, expected) in tasks {
        let req = ga_query::minimal_context::MinimalContextRequest::for_symbol(sym, 2000);
        match ga_query::minimal_context::minimal_context(&store, &req) {
            Ok(resp) => {
                let actual: std::collections::BTreeSet<&str> =
                    resp.symbols.iter().map(|s| s.file.as_str()).collect();
                let hits: Vec<&&str> = expected.iter().filter(|f| actual.contains(*f)).collect();
                let missed: Vec<&&str> = expected.iter().filter(|f| !actual.contains(*f)).collect();
                println!(
                    "{sym}: hits {}/{}, returned_files={} budget={:.2}",
                    hits.len(),
                    expected.len(),
                    actual.len(),
                    resp.budget_used
                );
                if !missed.is_empty() {
                    println!("    MISSED: {:?}", missed);
                }
                let sample: Vec<&str> = actual.iter().take(6).copied().collect();
                println!("    returned files (first 6): {:?}", sample);
            }
            Err(e) => println!("{sym}: ERROR {e}"),
        }
    }
}
