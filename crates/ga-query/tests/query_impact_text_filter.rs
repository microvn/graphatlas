//! EXP-M2-TEXTFILTER — post-BFS text-intersect filter. Drops impacted files
//! whose content doesn't mention the seed symbol as a word-boundary token.
//! Based on validate harness `m2_text_intersect_validate.rs` that showed
//! precision +0.205 / completeness -0.043 on dev corpus.
//!
//! Contract update 2026-05-03 per AS-016 investigation option (b):
//! filter is multi-token. Files survive when their text contains ANY
//! identifier from the BFS visited set as a word-boundary token (path
//! symbols), not only the seed name. Filter applies on every mode
//! (symbol-direct AND changed_files). AS-016 chain alpha ← beta ←
//! gamma is preserved because c.py contains `beta`/`gamma` (path
//! symbols). Hub noise reached via KG-9 sibling-method but mentioning
//! zero path symbols is dropped.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// Drive impact via `changed_files` so the text filter still runs (multi
/// branch). Used by tests that pin filter behavior.
fn run_via_changed_files(store: &Store, files: &[&str]) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            changed_files: Some(files.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// Drive impact via direct `symbol` — the AS-016 path. Filter is NOT
/// applied; full BFS output surfaces.
fn run_via_symbol(store: &Store, symbol: &str) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            symbol: Some(symbol.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

fn paths(resp: &ga_query::ImpactResponse) -> Vec<String> {
    resp.impacted_files.iter().map(|f| f.path.clone()).collect()
}

#[test]
fn multi_token_filter_keeps_call_chain_files_via_changed_files() {
    // Same fixture as the legacy single-token test, contract flipped:
    // under multi-token (option (b)) helper.py and deep.py SURVIVE
    // because their text contains the path symbols `helper` and
    // `deep_call` accumulated by BFS from `special_func`. The legit
    // call chain is no longer mistaken for hub noise.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("a.py"),
        "from helper import helper\n\ndef special_func():\n    helper()\n",
    );
    write(
        &repo.join("helper.py"),
        "from deep import deep_call\n\ndef helper():\n    deep_call()\n",
    );
    write(&repo.join("deep.py"), "def deep_call():\n    pass\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_via_changed_files(&store, &["a.py"]);
    let p = paths(&resp);
    assert!(p.contains(&"a.py".to_string()), "seed file: {:?}", p);
    assert!(
        p.contains(&"helper.py".to_string()),
        "helper.py contains path symbol `helper` — multi-token must keep: {:?}",
        p,
    );
    assert!(
        p.contains(&"deep.py".to_string()),
        "deep.py contains path symbol `deep_call` — multi-token must keep: {:?}",
        p,
    );
}

#[test]
fn text_filter_keeps_seed_depth_zero_even_if_name_missing_in_source() {
    // Edge case: seed defined in a generated / unusual file where textual
    // match might fail. depth=0 files must always survive regardless of
    // text-contains check. (Symbol-direct mode skips filter entirely so
    // the depth-0 invariant trivially holds; this test pins it under the
    // most permissive path so future filter changes can't strip the seed
    // file even if they re-enable filtering on the symbol path.)
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("defs.py"), "def mymethod():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_via_symbol(&store, "mymethod");
    let p = paths(&resp);
    assert!(
        p.contains(&"defs.py".to_string()),
        "seed definition file always in output: {:?}",
        p,
    );
}

#[test]
fn multi_token_filter_word_boundary_drops_substring_only_via_changed_files() {
    // Word-boundary semantics still apply per-symbol: a file mentioning
    // ONLY substrings of path symbols (no full token) is dropped.
    // Fixture: seed `foo`. Direct caller `real.py` has `foo()` token →
    // kept. `decoy.py` is a sibling reachable via REFERENCES that ONLY
    // contains substring `fooprint` (not `foo` token) and no other path
    // symbol — must be dropped.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(&repo.join("a.py"), "def foo():\n    pass\n");
    write(
        &repo.join("real.py"),
        "from a import foo\ndef call_it():\n    foo()\n",
    );
    // Substring-only file: text contains "fooprint" (and "size" comment)
    // but neither `foo`, `call_it`, nor any other path symbol as a
    // word-boundary token. Reaches impact via REFERENCES on the seed.
    write(
        &repo.join("decoy.py"),
        "from a import foo as fooprint\n\
         x = fooprint\n\
         # unrelated comment: size\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_via_changed_files(&store, &["a.py"]);
    let p = paths(&resp);
    assert!(p.contains(&"a.py".to_string()));
    assert!(
        p.contains(&"real.py".to_string()),
        "real.py has foo() token (path symbol): {:?}",
        p
    );
    assert!(
        !p.contains(&"decoy.py".to_string()),
        "decoy.py mentions only `fooprint` substring (no `foo`/`call_it` \
         tokens) — multi-token must drop: {:?}",
        p,
    );
}

#[test]
fn symbol_direct_mode_preserves_transitive_references_chain() {
    // AS-016 regression guard at the ga-query crate level (closer to the
    // bug than the MCP e2e). Chain: a.py.alpha → b.py.beta (REFERENCES)
    // → c.py.gamma (REFERENCES). c.py never textually mentions `alpha` —
    // the pre-fix filter unconditionally dropped it. Option (a) skips
    // text filter when the caller drives via `symbol`, so the full chain
    // surfaces.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(&repo.join("a.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("b.py"),
        "from a import alpha\n\ndef beta():\n    m = {'k': alpha}\n",
    );
    write(
        &repo.join("c.py"),
        "from b import beta\n\ndef gamma():\n    m = {'k': beta}\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("alpha".into()),
            max_depth: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    let p = paths(&resp);

    assert!(p.contains(&"a.py".to_string()), "seed file: {:?}", p);
    assert!(
        p.contains(&"b.py".to_string()),
        "depth-1 REFERENCES file b.py must surface: {:?}",
        p
    );
    assert!(
        p.contains(&"c.py".to_string()),
        "AS-016: depth-2 REFERENCES file c.py must surface even though \
         its source text never mentions `alpha` (filter must be skipped \
         in symbol-direct mode): {:?}",
        p
    );
}

// === Multi-token filter (option (b) from AS-016 investigation) ===
//
// Contract update 2026-05-03: filter accepts a file if its text contains
// ANY symbol on the BFS path from seed to that file (the "path symbols"
// set), not only the seed name. Restores filter on symbol-direct mode
// while keeping AS-016 invariant: c.py contains intermediate `beta`,
// which is a path symbol, so c.py survives even though `alpha` is absent.

#[test]
fn multi_token_filter_keeps_legit_chain_files_on_symbol_direct() {
    // a.py defines special_func → calls helper. helper → calls deep_call.
    // Under single-token filter, helper.py and deep.py would be dropped
    // (text doesn't say "special_func"). Under multi-token, they survive
    // because their text contains intermediate path symbols (`helper`,
    // `deep_call`) — those are legitimate call-chain hops, not hub noise.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("a.py"),
        "from helper import helper\n\ndef special_func():\n    helper()\n",
    );
    write(
        &repo.join("helper.py"),
        "from deep import deep_call\n\ndef helper():\n    deep_call()\n",
    );
    write(&repo.join("deep.py"), "def deep_call():\n    pass\n");

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_via_symbol(&store, "special_func");
    let p = paths(&resp);
    assert!(p.contains(&"a.py".to_string()), "seed file: {:?}", p);
    assert!(
        p.contains(&"helper.py".to_string()),
        "helper.py is on the BFS path (depth-1 callee), text contains \
         path symbol `helper` and `deep_call` — multi-token must keep: {:?}",
        p,
    );
    assert!(
        p.contains(&"deep.py".to_string()),
        "deep.py is on the BFS path (depth-2 callee), text contains \
         path symbol `deep_call` — multi-token must keep: {:?}",
        p,
    );
}

#[test]
fn multi_token_filter_drops_noise_with_no_path_symbol_mention_on_symbol_direct() {
    // KG-9 sibling-method walk pulls files into the impact set at depth=2
    // confidence=0.7 even when the seed never references them — only a
    // sibling-by-CONTAINS does. If that sibling's call target lives in a
    // file that mentions ZERO symbols on the BFS path, the file is pure
    // noise (cardshield class: paypal.ts surfaced under loadEndpoint).
    // Multi-token filter must drop these.
    //
    // Fixture: class Service has alpha (seed) and beta (sibling). beta
    // calls unrelated_helper in noise.py. noise.py text mentions only
    // `unrelated_helper` and `noise_helper` — none of {alpha, beta, Service}.
    // BFS forward from alpha is empty (alpha calls nothing). KG-9 inserts
    // noise.py at depth=2. After multi-token filter: noise.py dropped.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("main.py"),
        "from noise import unrelated_helper\n\
         class Service:\n\
         \x20   def alpha(self):\n\
         \x20       pass\n\
         \x20   def beta(self):\n\
         \x20       unrelated_helper()\n",
    );
    write(
        &repo.join("noise.py"),
        "def unrelated_helper():\n    noise_helper()\n\n\
         def noise_helper():\n    pass\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run_via_symbol(&store, "alpha");
    let p = paths(&resp);
    assert!(p.contains(&"main.py".to_string()), "seed file: {:?}", p);
    assert!(
        !p.contains(&"noise.py".to_string()),
        "noise.py is KG-9 sibling-method noise; text contains no path \
         symbol from BFS visited set — multi-token filter must drop: {:?}",
        p,
    );
}
