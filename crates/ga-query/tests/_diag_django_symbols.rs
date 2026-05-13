//! One-off diagnostic — query django's bench-cache for the 6 tasks-v6
//! task target symbols. Confirms which are findable in ga's index and
//! which return None from `find_symbol_def`. Run:
//!   cargo test -p ga-query --test _diag_django_symbols -- --nocapture --ignored
//! After a `bench --gate m3 --uc minimal_context --fixture django` to
//! ensure the cache exists.

use ga_index::Store;
use std::path::PathBuf;

#[test]
#[ignore]
fn diag_gin_minimal_context() {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let cache = repo_root.join(".graphatlas-bench-cache/m3-minimal_context/gin/ga");
    let fixture = repo_root.join("benches/fixtures/gin");
    let _ = std::fs::remove_dir_all(&cache);
    let store = Store::open_with_root(&cache, &fixture).expect("open store");
    ga_query::indexer::build_index(&store, &fixture).expect("build_index");

    let task_data: Vec<(&str, Vec<&str>)> = vec![
        (
            "TestContextRenderPDF",
            vec![
                "TestContextRenderPDF",
                "TestContextRenderNoContentPDF",
                "TestRenderPDF",
            ],
        ),
        (
            "TestContextGetError",
            vec!["TestContextGetError", "TestContextGetErrorSlice"],
        ),
        (
            "trySetUsingParser",
            vec![
                "trySetUsingParser",
                "setWithProperType",
                "setArray",
                "setSlice",
            ],
        ),
    ];
    for (sym, must) in task_data {
        let req = ga_query::minimal_context::MinimalContextRequest::for_symbol(sym, 2000);
        match ga_query::minimal_context::minimal_context(&store, &req) {
            Ok(resp) => {
                let returned: Vec<&str> = resp.symbols.iter().map(|s| s.symbol.as_str()).collect();
                let hits: Vec<&&str> = must.iter().filter(|m| returned.contains(m)).collect();
                println!(
                    "{sym}: returned={:?} hits {}/{} budget_used={}",
                    returned,
                    hits.len(),
                    must.len(),
                    resp.budget_used
                );
            }
            Err(e) => println!("{sym}: ERROR {e}"),
        }
    }
}

#[test]
#[ignore]
fn diag_find_django_targets_in_bench_cache() {
    let repo_root = PathBuf::from("/Volumes/Data/projects/me/graphatlas");
    let cache = repo_root.join(".graphatlas-bench-cache/m3-minimal_context/django/ga");
    let fixture = repo_root.join("benches/fixtures/django");
    if !cache.is_dir() {
        eprintln!("SKIP — no cache at {cache:?}; run bench first");
        return;
    }

    // Force rebuild — bench runs wipe cache between runs and the
    // "previous index was incomplete" recovery isn't running build_index,
    // it just opens an empty store.
    let _ = std::fs::remove_dir_all(&cache);
    let store = Store::open_with_root(&cache, &fixture).expect("open store");
    ga_query::indexer::build_index(&store, &fixture).expect("build_index");

    // Sanity — count total symbols in graph.
    let conn0 = store.connection().expect("connection");
    let total_rs = conn0
        .query("MATCH (s:Symbol) RETURN count(s)")
        .expect("count");
    let total: i64 = total_rs
        .into_iter()
        .next()
        .and_then(|row| row.into_iter().next())
        .and_then(|v| match v {
            lbug::Value::Int64(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0);
    println!("\n=== TOTAL SYMBOLS IN GRAPH: {total} ===");
    drop(conn0);

    let targets = [
        ("user_perm_str", "django/contrib/auth/models.py"),
        (
            "test_when_rejects_invalid_arguments",
            "django/db/models/expressions.py",
        ),
        (
            "_check_conflict_with_managers",
            "django/db/models/fields/related.py",
        ),
        ("my_task", "django/tasks/base.py"),
        ("_check_data_too_big", "django/http/request.py"),
        ("has_add_permission", "django/contrib/admin/options.py"),
    ];

    let conn = store.connection().expect("connection");

    println!("\n=== Per-symbol diagnosis ===");
    for (sym, expected_file) in &targets {
        // Exact-name match — same query find_symbol_def uses.
        let cypher_exact = format!(
            "MATCH (s:Symbol) WHERE s.name = '{sym}' AND s.kind <> 'external' \
             RETURN s.name, s.file, s.line, s.kind"
        );
        let exact: Vec<(String, String, i64, String)> = conn
            .query(&cypher_exact)
            .map(|rs| {
                rs.into_iter()
                    .filter_map(|row| {
                        let cols: Vec<lbug::Value> = row.into_iter().collect();
                        if cols.len() < 4 {
                            return None;
                        }
                        let name = match &cols[0] {
                            lbug::Value::String(s) => s.clone(),
                            _ => return None,
                        };
                        let file = match &cols[1] {
                            lbug::Value::String(s) => s.clone(),
                            _ => return None,
                        };
                        let line = match &cols[2] {
                            lbug::Value::Int64(n) => *n,
                            _ => 0,
                        };
                        let kind = match &cols[3] {
                            lbug::Value::String(s) => s.clone(),
                            _ => return None,
                        };
                        Some((name, file, line, kind))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // File-scoped: maybe symbol exists at expected path but as a
        // different kind / nested name.
        let cypher_file = format!(
            "MATCH (s:Symbol) WHERE s.file = '{expected_file}' \
             RETURN s.name, s.kind, s.line"
        );
        let in_file: Vec<(String, String, i64)> = conn
            .query(&cypher_file)
            .map(|rs| {
                rs.into_iter()
                    .filter_map(|row| {
                        let cols: Vec<lbug::Value> = row.into_iter().collect();
                        if cols.len() < 3 {
                            return None;
                        }
                        let name = match &cols[0] {
                            lbug::Value::String(s) => s.clone(),
                            _ => return None,
                        };
                        let kind = match &cols[1] {
                            lbug::Value::String(s) => s.clone(),
                            _ => return None,
                        };
                        let line = match &cols[2] {
                            lbug::Value::Int64(n) => *n,
                            _ => 0,
                        };
                        Some((name, kind, line))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let in_file_names: Vec<&str> = in_file.iter().map(|(n, _, _)| n.as_str()).collect();
        let target_in_file = in_file_names.contains(sym);

        println!("\n--- TARGET: {sym} (expected_file: {expected_file}) ---");
        println!("  exact-name matches: {} entries", exact.len());
        for e in exact.iter().take(3) {
            println!("    - {}::{} line={} kind={}", e.1, e.0, e.2, e.3);
        }
        println!(
            "  in expected_file: {} symbols total, target present: {}",
            in_file.len(),
            target_in_file
        );
        if !target_in_file && !in_file_names.is_empty() {
            // Show 5 names to see what indexer DID extract from this file
            let sample: Vec<&str> = in_file_names.iter().take(5).copied().collect();
            println!("    (file has e.g.: {:?})", sample);
        }
    }
    println!("\n=== End ===\n");

    // ─────────────────────────────────────────────────────────────────
    // Per-task minimal_context output vs must_touch_symbols GT
    // ─────────────────────────────────────────────────────────────────
    println!("\n=== minimal_context per-task recall ===");
    let task_data: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "django-e2abe321",
            "user_perm_str",
            vec!["user_perm_str", "test_user_perm_str"],
        ),
        (
            "django-3b161e60",
            "test_when_rejects_invalid_arguments",
            vec!["When", "test_when_rejects_invalid_arguments"],
        ),
        (
            "django-fcf91688",
            "_check_conflict_with_managers",
            vec![
                "_check_conflict_with_managers",
                "test_clash_managers_related_name",
            ],
        ),
        (
            "django-e27f23b2",
            "my_task",
            vec!["task", "test_task_kwargs"],
        ),
        (
            "django-953c2380",
            "_check_data_too_big",
            vec!["_check_data_too_big", "test_check_data_too_big"],
        ),
        (
            "django-6afe7ce9",
            "has_add_permission",
            vec!["has_add_permission", "test_has_add_permission"],
        ),
    ];
    for (task_id, sym, must_touch_symbols) in task_data {
        let req = ga_query::minimal_context::MinimalContextRequest::for_symbol(sym, 2000);
        match ga_query::minimal_context::minimal_context(&store, &req) {
            Ok(resp) => {
                let returned: std::collections::BTreeSet<String> =
                    resp.symbols.iter().map(|s| s.symbol.clone()).collect();
                let hits: Vec<&&str> = must_touch_symbols
                    .iter()
                    .filter(|m| returned.contains(**m))
                    .collect();
                println!(
                    "{task_id} sym={sym}: returned {} symbols, hits {}/{} must_touch_symbols={:?} | hit={:?}",
                    resp.symbols.len(),
                    hits.len(),
                    must_touch_symbols.len(),
                    must_touch_symbols,
                    hits
                );
                println!(
                    "    sample returned: {:?}",
                    resp.symbols
                        .iter()
                        .take(8)
                        .map(|s| s.symbol.as_str())
                        .collect::<Vec<_>>()
                );
            }
            Err(e) => {
                println!("{task_id} sym={sym}: ERROR {e}");
            }
        }
    }
    println!("=== End per-task ===\n");
}
