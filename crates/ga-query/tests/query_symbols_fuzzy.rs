//! Tools S-004 cluster B — ga_symbols fuzzy Levenshtein mode (AS-009).

use ga_index::Store;
use ga_query::{indexer::build_index, symbols, SymbolsMatch};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

#[test]
fn fuzzy_ranks_levenshtein_close_first() {
    // AS-009: pattern "usr_srlzr" should rank "UserSerializer" near the top.
    // In fuzzy mode we allow non-identifier wildcards; but pattern is still an
    // ident here (underscore allowed).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def UserSerializer(): pass\ndef TotallyUnrelated(): pass\ndef AnotherThing(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "UsrSrlzr", SymbolsMatch::Fuzzy).unwrap();
    assert!(!resp.symbols.is_empty(), "fuzzy must surface candidates");
    assert_eq!(
        resp.symbols[0].name, "UserSerializer",
        "Levenshtein-closest result first"
    );
}

#[test]
fn fuzzy_empty_pattern_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def ok(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "", SymbolsMatch::Fuzzy).unwrap();
    assert!(resp.symbols.is_empty());
}

#[test]
fn fuzzy_caps_at_ten() {
    // 15 defs + fuzzy → still capped at 10, meta reflects total.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    for i in 0..15 {
        write(&repo.join(format!("f{i}.py")), "def alpha(): pass\n");
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "alpha", SymbolsMatch::Fuzzy).unwrap();
    assert_eq!(resp.symbols.len(), 10);
    assert!(resp.meta.truncated);
    assert_eq!(resp.meta.total_available, 15);
}

#[test]
fn fuzzy_score_monotonic_with_distance() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def alpha(): pass\ndef alphaz(): pass\ndef zzzzzz(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "alpha", SymbolsMatch::Fuzzy).unwrap();
    // Closer match must have >= score of farther match.
    let alpha = resp.symbols.iter().find(|s| s.name == "alpha").unwrap();
    let zzzzzz = resp.symbols.iter().find(|s| s.name == "zzzzzz").unwrap();
    assert!(alpha.score >= zzzzzz.score);
}
