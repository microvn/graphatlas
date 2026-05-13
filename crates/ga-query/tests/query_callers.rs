//! Tools S-001 cluster C — AS-001 happy path: ga_callers returns direct
//! callers of a symbol within the indexed graph.

use ga_index::Store;
use ga_query::{callers, indexer::build_index};
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
fn all_direct_callers() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller_a():\n    target()\n\ndef caller_b():\n    target()\n\ndef caller_c():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert_eq!(resp.callers.len(), 3);
    let mut names: Vec<String> = resp.callers.iter().map(|c| c.symbol.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "caller_a".to_string(),
            "caller_b".to_string(),
            "caller_c".to_string()
        ]
    );
    for c in &resp.callers {
        assert_eq!(c.file, "m.py");
    }
    assert!(resp.meta.symbol_found);
}

#[test]
fn filters_by_file() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\ndef caller_a():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\ndef caller_b():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // AS-003 changed semantics: file filter narrows the EXACT def (confidence
    // 1.0) but polymorphic same-name defs in other files still surface at 0.6.
    let resp = callers(&store, "target", Some("a.py")).unwrap();
    let exact: Vec<_> = resp
        .callers
        .iter()
        .filter(|c| (c.confidence - 1.0).abs() < 1e-6)
        .collect();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].symbol, "caller_a");
    assert_eq!(exact[0].file, "a.py");
}

#[test]
fn empty_when_no_callers() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def lonely(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "lonely", None).unwrap();
    assert!(resp.callers.is_empty(), "{:?}", resp.callers);
    assert!(resp.meta.symbol_found, "symbol 'lonely' exists in graph");
}

#[test]
fn returns_call_site_line() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // Lines: 1=def target, 2=blank, 3=def caller, 4=    target()
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert_eq!(resp.callers.len(), 1);
    assert_eq!(resp.callers[0].call_site_line, 4);
}

#[test]
fn returns_caller_definition_line() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert_eq!(resp.callers.len(), 1);
    assert_eq!(resp.callers[0].line, 3, "caller defined at line 3");
}

#[test]
fn handles_quote_in_symbol_safely() {
    // Attacker / malformed MCP input: symbol name contains a single quote.
    // Must not break the Cypher query or panic — should return empty results.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def target(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "tar'get", None).unwrap();
    assert!(resp.callers.is_empty());
    assert!(!resp.meta.symbol_found);
    assert!(resp.meta.suggestion.is_empty());
}
