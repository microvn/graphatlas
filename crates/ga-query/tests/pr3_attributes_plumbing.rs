//! v1.3 PR3 — Parser → indexer plumbing for SymbolAttribute + confidence.
//!
//! Spec: spec, S-004.
//! Plan: v4 migration plan, PR3 row.
//!
//! Scope:
//! - AS-010 (mapping): SymbolAttribute::{Async,Override,Static} → bool cols.
//!   Per-lang DETECTION of async / abstract is PR4 scope; PR3 only wires the
//!   mapping so once a lang spec emits SymbolAttribute::Async, the bool fires.
//! - AS-011: Ruby `define_method` confidence = 0.6 surfaces in DB (broken-
//!   promise fix from walker.rs:44-49 / Tools-C11).
//! - is_generated denormalized from `confidence < 1.0` (synthetic symbols).
//!
//! Out of scope: AS-012 `is_test_marker` (deferred — needs new per-lang
//! parser hook to detect `#[test]` / `@Test` / pytest naming; not "stop
//! dropping" per PR3 plan §F).

use ga_index::Store;
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

#[test]
fn as_011_ruby_define_method_confidence_is_0_6() {
    // AS-011 — Ruby `define_method` synthetic symbol carries confidence 0.6
    // per Tools-C11. v3 dropped this; v4 must surface it in the Symbol row.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.rb",
        "class C\n  define_method(:dyn_foo) { puts \"hi\" }\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {name: 'dyn_foo'}) RETURN s.confidence")
        .expect("query confidence");
    let mut got: Option<f64> = None;
    for row in rs {
        match row.into_iter().next() {
            Some(lbug::Value::Double(f)) => got = Some(f),
            Some(lbug::Value::Float(f)) => got = Some(f as f64),
            _ => {}
        }
    }
    let v = got.expect("dyn_foo Symbol row missing or confidence not numeric");
    assert!(
        (v - 0.6).abs() < 1e-3,
        "AS-011 broken-promise fix: Ruby define_method confidence must be 0.6, got {v}"
    );
}

#[test]
fn as_011_define_method_is_generated_true() {
    // AS-010 derived rule: synthetic symbols (confidence < 1.0) → is_generated.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.rb",
        "class C\n  define_method(:dyn_bar) { puts \"hi\" }\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {name: 'dyn_bar'}) RETURN s.is_generated")
        .expect("query is_generated");
    let mut got = false;
    let mut saw_row = false;
    for row in rs {
        saw_row = true;
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            got = b;
        }
    }
    assert!(saw_row, "no Symbol row for dyn_bar");
    assert!(got, "synthetic symbol must have is_generated == true");
}

#[test]
fn ordinary_symbol_confidence_is_1_0() {
    // Universal default: every parsed symbol has confidence = 1.0 unless the
    // parser explicitly downgrades it (Ruby define_method = 0.6). Catches
    // accidental uniform-zero / uniform-default-low regressions.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "main.py", "def hello():\n    return 1\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {name: 'hello'}) RETURN s.confidence, s.is_generated")
        .unwrap();
    let mut conf: Option<f64> = None;
    let mut gen: Option<bool> = None;
    for row in rs {
        let mut it = row.into_iter();
        match it.next() {
            Some(lbug::Value::Double(f)) => conf = Some(f),
            Some(lbug::Value::Float(f)) => conf = Some(f as f64),
            _ => {}
        }
        if let Some(lbug::Value::Bool(b)) = it.next() {
            gen = Some(b);
        }
    }
    assert!(
        (conf.unwrap_or(-1.0) - 1.0).abs() < 1e-3,
        "ordinary symbol must carry confidence 1.0, got {conf:?}"
    );
    assert_eq!(
        gen,
        Some(false),
        "ordinary symbol is_generated must be false"
    );
}

#[test]
fn as_010_default_attribute_bools_are_false() {
    // Symbols without SymbolAttribute markers must have all 5 denormalized
    // bools = false. Catches accidental "true-by-default" wiring.
    let repo = TempDir::new().unwrap();
    write_file(repo.path(), "main.py", "def plain():\n    return 1\n");
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (s:Symbol {name: 'plain'}) \
             RETURN s.is_async, s.is_override, s.is_abstract, s.is_static, s.is_test_marker",
        )
        .unwrap();
    let mut bools = Vec::new();
    for row in rs {
        for v in row {
            if let lbug::Value::Bool(b) = v {
                bools.push(b);
            }
        }
    }
    assert_eq!(bools.len(), 5, "expected 5 bool cols, got {bools:?}");
    for (i, b) in bools.iter().enumerate() {
        assert!(!b, "default bool col[{i}] must be false");
    }
}

#[test]
fn kotlin_suspend_does_not_set_is_async() {
    // Kotlin `suspend` is currently the ONLY SymbolAttribute populated by an
    // existing per-lang spec (kotlin.rs:96). It maps to `Suspend`, NOT
    // `Async` — they're separate variants. `is_async` must stay false; PR3
    // plumbs the mapping but doesn't conflate semantics. Suspend-specific
    // column is out of v4 scope.
    //
    // This test guards against a tempting "is_async = attrs.iter().any(…)"
    // shortcut that lumps Suspend into Async.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.kt",
        "package x\nsuspend fun do_work(): Int { return 1 }\n",
    );
    let (_t, store) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol {name: 'do_work'}) RETURN s.is_async")
        .unwrap();
    let mut got: Option<bool> = None;
    for row in rs {
        if let Some(lbug::Value::Bool(b)) = row.into_iter().next() {
            got = Some(b);
        }
    }
    // If Kotlin parser didn't pick up the symbol, skip rather than fail —
    // the contract is "Suspend doesn't bleed into is_async", which only
    // matters when a row exists.
    if let Some(b) = got {
        assert!(
            !b,
            "Kotlin Suspend must NOT be mapped to is_async (separate semantics)"
        );
    }
}
