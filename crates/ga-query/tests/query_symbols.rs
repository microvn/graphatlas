//! Tools S-004 cluster A — ga_symbols exact match + AS-010 IndexNotReady.

use ga_core::Error;
use ga_index::Store;
use ga_query::{indexer::build_index, symbols, SymbolsMatch};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn exact_match_returns_hit() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def UserSerializer(): pass\ndef OtherFn(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "UserSerializer", SymbolsMatch::Exact).unwrap();
    assert_eq!(resp.symbols.len(), 1);
    assert_eq!(resp.symbols[0].name, "UserSerializer");
    assert_eq!(resp.symbols[0].file, "m.py");
}

#[test]
fn exact_match_ranks_by_caller_count() {
    // AS-008: "ranked by relevance (callers count boost)". The symbol with
    // more callers should come first when multiple files define same name.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // `target` defined in a.py has 2 callers, in b.py has 0 callers.
    write(
        &repo.join("a.py"),
        "def target(): pass\ndef ca():\n    target()\ndef cb():\n    target()\n",
    );
    write(&repo.join("b.py"), "def target(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "target", SymbolsMatch::Exact).unwrap();
    assert_eq!(resp.symbols.len(), 2);
    // Higher-caller-count def ranked first.
    assert_eq!(resp.symbols[0].file, "a.py");
    assert!(resp.symbols[0].score >= resp.symbols[1].score);
}

#[test]
fn exact_match_no_hit_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def something(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "nonexistent", SymbolsMatch::Exact).unwrap();
    assert!(resp.symbols.is_empty());
}

#[test]
fn caps_output_at_ten_results() {
    // AS-008: Response ≤10 results. Populate 15 same-name defs across files.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    for i in 0..15 {
        write(&repo.join(format!("f{i}.py")), "def shared(): pass\n");
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "shared", SymbolsMatch::Exact).unwrap();
    assert_eq!(resp.symbols.len(), 10);
    assert!(resp.meta.truncated);
    assert_eq!(resp.meta.total_available, 15);
}

#[test]
fn empty_index_returns_not_ready_error() {
    // AS-010: Fresh index still building → -32000 IndexNotReady.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // No build_index call — graph is empty.
    let store = Store::open_with_root(&cache, &repo).unwrap();

    let err = symbols(&store, "anything", SymbolsMatch::Exact).unwrap_err();
    match err {
        Error::IndexNotReady { status, .. } => assert_eq!(status, "indexing"),
        other => panic!("expected IndexNotReady, got {other:?}"),
    }
}

#[test]
fn empty_pattern_returns_empty() {
    // Tools-C9-d: is_safe_ident("") is false → empty response (not error).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def ok(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "", SymbolsMatch::Exact).unwrap();
    assert!(resp.symbols.is_empty());
    assert_eq!(resp.meta.total_available, 0);
}

#[test]
fn does_not_surface_external_symbols() {
    // External synthetic Symbol nodes are created for stdlib / third-party
    // callees (kind = 'external'). ga_symbols must exclude them — they aren't
    // real source defs in the indexed repo.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "import hashlib\ndef run():\n    hashlib.sha256()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // hashlib / sha256 / methods on stdlib would be indexed as external —
    // ga_symbols must not return them.
    let resp = symbols(&store, "sha256", SymbolsMatch::Exact).unwrap();
    assert!(
        resp.symbols.is_empty(),
        "external synthetic symbols must not surface: {:?}",
        resp.symbols
    );
}

#[test]
fn rejects_non_safe_pattern() {
    // Tools-C9-d identifier allowlist on `pattern`.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def ok(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "ok'; DROP", SymbolsMatch::Exact).unwrap();
    assert!(resp.symbols.is_empty());
}
