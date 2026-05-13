//! Gap 7 / Fix A — extend `ga_hubs` edge-kind list to count v4 RELs.
//!
//! Adds 4 edge kinds previously ignored by the hub-degree query:
//! - `IMPLEMENTS` (Symbol→Symbol): trait/interface gets in-degree per impl
//! - `DECORATES` (Symbol→Symbol): decorator out-deg + decorated in-deg
//! - `IMPORTS_NAMED` (File→Symbol): Symbol in-deg = importing files
//! - `MODULE_TYPED` (File→Symbol): Symbol in-deg = type-position uses
//!
//! Pure reuse of v4 schema — no new edges emitted. Universal-truth: hub =
//! structural centrality measured by ALL incident edges, not just CALLS-family.
//! Iron-rule retained: CALLS_HEURISTIC default-exclude (Tools-C6) unchanged.

use ga_index::Store;
use ga_query::hubs::{hubs, HubsEdgeTypes, HubsRequest};
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn lookup_in_degree(store: &Store, name: &str) -> u32 {
    let req = HubsRequest {
        top_n: 100,
        symbol: Some(name.to_string()),
        file: None,
        edge_types: HubsEdgeTypes::Default,
    };
    let resp = hubs(store, &req).unwrap();
    resp.hubs
        .iter()
        .find(|h| h.name == name)
        .map(|h| h.in_degree)
        .unwrap_or(0)
}

#[test]
fn implements_edge_contributes_to_in_degree() {
    // Java interface I implemented by C → I's in_degree should increment.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "I.java", "interface I { void run(); }\n");
    write_file(
        repo.path(),
        "C.java",
        "class C implements I { public void run() {} }\n",
    );
    write_file(
        repo.path(),
        "D.java",
        "class D implements I { public void run() {} }\n",
    );
    let (_t, store) = index_repo(repo.path());
    // Pre-Fix-A: only EXTENDS counted → 2. Post-Fix-A: EXTENDS (2) + IMPLEMENTS (2) = 4.
    let n = lookup_in_degree(&store, "I");
    assert!(
        n >= 4,
        "I should have ≥4 in-degree (2 EXTENDS + 2 IMPLEMENTS); got {n}"
    );
}

#[test]
fn decorates_edge_contributes_to_in_degree() {
    // @my_dec on f1 + f2 + f3 → my_dec out_degree += 3, each decorated +=1.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "main.py",
        "def my_dec(fn): return fn\n\n\
         @my_dec\ndef f1(): pass\n\n\
         @my_dec\ndef f2(): pass\n\n\
         @my_dec\ndef f3(): pass\n",
    );
    let (_t, store) = index_repo(repo.path());
    let req = HubsRequest {
        top_n: 100,
        symbol: Some("my_dec".to_string()),
        file: None,
        edge_types: HubsEdgeTypes::Default,
    };
    let resp = hubs(&store, &req).unwrap();
    let my_dec = resp
        .hubs
        .iter()
        .find(|h| h.name == "my_dec")
        .expect("my_dec must rank");
    assert!(
        my_dec.out_degree >= 3,
        "my_dec out_degree must include 3 DECORATES; got {}",
        my_dec.out_degree
    );
}

#[test]
fn imports_named_contributes_to_in_degree() {
    // a.py defines `helper`. b.py + c.py + d.py all import helper → helper
    // in_degree += 3 (one per importing file).
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(
        repo.path(),
        "b.py",
        "from a import helper\ndef use_b(): return helper()\n",
    );
    write_file(
        repo.path(),
        "c.py",
        "from a import helper\ndef use_c(): return helper()\n",
    );
    write_file(
        repo.path(),
        "d.py",
        "from a import helper\ndef use_d(): return helper()\n",
    );
    let (_t, store) = index_repo(repo.path());
    // Pre-Fix-A: helper in_degree only counts Symbol→Symbol CALLS (3). Post-Fix-A:
    // also +3 from IMPORTS_NAMED (b/c/d.py → helper).
    let n = lookup_in_degree(&store, "helper");
    assert!(
        n >= 6,
        "helper should have ≥6 in-degree (3 CALLS + 3 IMPORTS_NAMED); got {n}"
    );
}

#[test]
fn module_typed_contributes_to_in_degree() {
    // Rust type-position references at module scope (not inside a fn) emit
    // MODULE_TYPED edges. Pre-Fix-A: ignored. Post-Fix-A: count toward the
    // referenced Symbol's in-degree.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "pub trait Greet { fn hi(&self); }\n\
         pub struct Foo;\n\
         pub struct Bar;\n\
         pub static G: &dyn Greet = &Foo;\n\
         pub static H: &dyn Greet = &Bar;\n",
    );
    let (_t, store) = index_repo(repo.path());
    // Greet referenced at module scope twice (G, H) → MODULE_TYPED edges.
    // Pre-Fix-A: 0 contribution (module_typed not in hub query). Post-Fix-A:
    // Greet's in_degree includes MODULE_TYPED edges.
    let n = lookup_in_degree(&store, "Greet");
    // Soft assertion — exact MODULE_TYPED count depends on parser ref_kind.
    // We only need to verify it's *non-zero* contribution beyond the
    // (default 0) pre-fix baseline.
    assert!(
        n >= 1,
        "Greet should have ≥1 in-degree from MODULE_TYPED type-position refs; got {n}"
    );
}

#[test]
fn calls_heuristic_still_excluded_in_default_mode() {
    // Regression check: Fix A must not break Tools-C6 invariant.
    // Cross-file no-import call → tier-3 heuristic. Default mode subtracts.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "a.py", "def helper():\n    return 1\n");
    write_file(repo.path(), "b.py", "def b1():\n    return helper()\n");
    let (_t, store) = index_repo(repo.path());
    let n = lookup_in_degree(&store, "helper");
    // Pre-Fix-A: 0 (heuristic subtracted, no other contribution).
    // Post-Fix-A: 0 still (Tools-C6 default-exclude unchanged); since b.py
    // has no import statement, no IMPORTS_NAMED contribution either.
    assert_eq!(
        n, 0,
        "tier-3 heuristic edge must stay subtracted in default mode (no import = no IMPORTS_NAMED contrib either); got {n}"
    );
}
