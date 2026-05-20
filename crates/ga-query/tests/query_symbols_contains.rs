//! Spec E S-001 — SymbolsMatch::Contains. Case-insensitive substring with
//! prefix-priority ranking, intended for HTTP search-as-you-type
//! (cap = 50 per Spec E C-2).

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

// ---------- AS-001 ----------

#[test]
fn as001_contains_substring_case_insensitive() {
    // Pattern "connect" must surface all three names; case-insensitive.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def ConnectingFailedEventArgs(): pass\n\
         def OnConnect(): pass\n\
         def reconnect_handler(): pass\n\
         def unrelated(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "connect", SymbolsMatch::Contains).unwrap();
    let names: Vec<&str> = resp.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"ConnectingFailedEventArgs"), "names: {names:?}");
    assert!(names.contains(&"OnConnect"), "names: {names:?}");
    assert!(names.contains(&"reconnect_handler"), "names: {names:?}");
    assert!(!names.contains(&"unrelated"), "names: {names:?}");
}

#[test]
fn as001_contains_prefix_match_ranks_first() {
    // "connect" → ConnectingFailedEventArgs (prefix) before OnConnect / reconnect_handler (mid).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def ConnectingFailedEventArgs(): pass\n\
         def OnConnect(): pass\n\
         def reconnect_handler(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "connect", SymbolsMatch::Contains).unwrap();
    assert_eq!(
        resp.symbols.first().map(|s| s.name.as_str()),
        Some("ConnectingFailedEventArgs"),
        "prefix-match must come first; got {:?}",
        resp.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[test]
fn as001_contains_uppercase_pattern_still_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def OnConnect(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "CONNECT", SymbolsMatch::Contains).unwrap();
    assert_eq!(resp.symbols.len(), 1);
    assert_eq!(resp.symbols[0].name, "OnConnect");
}

#[test]
fn as001_contains_empty_pattern_returns_empty() {
    // is_safe_ident rejects empty — pattern "" → no query, empty response.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def ok(): pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "", SymbolsMatch::Contains).unwrap();
    assert!(resp.symbols.is_empty());
}

// ---------- AS-002 cap + truncated ----------

#[test]
fn as002_contains_caps_at_fifty_with_truncated_flag() {
    // 60 defs containing "alpha" → cap 50, truncated=true, total_available=60.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // 60 files each with one matching name; reuse-of-name across files is fine
    // because Symbol nodes are file-scoped.
    let mut src = String::new();
    for i in 0..60 {
        src.push_str(&format!("def alpha{i}(): pass\n"));
    }
    write(&repo.join("m.py"), &src);
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "alpha", SymbolsMatch::Contains).unwrap();
    assert_eq!(resp.symbols.len(), 50);
    assert!(resp.meta.truncated);
    assert_eq!(resp.meta.total_available, 60);
}

// ---------- AS-001 ordering: prefix-first then alpha ----------

#[test]
fn as001_contains_alpha_tiebreak_within_prefix_tier() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def connect_b(): pass\n\
         def connect_a(): pass\n\
         def Connect_C(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "connect", SymbolsMatch::Contains).unwrap();
    let names: Vec<&str> = resp.symbols.iter().map(|s| s.name.as_str()).collect();
    // All three are prefix matches; case-insensitive alphabetical order.
    assert_eq!(names, vec!["connect_a", "connect_b", "Connect_C"]);
}

// ---------- Edge: external kind filtered ----------

#[test]
fn contains_excludes_external_kind() {
    // External symbols (cluster-B unresolved imports) must not appear in
    // search results — they're not navigable in the UI.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "import socket\n\
         socket.connect()\n\
         def my_connect(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "connect", SymbolsMatch::Contains).unwrap();
    for s in &resp.symbols {
        assert_ne!(s.kind, "external", "external leaked: {s:?}");
    }
    // my_connect must appear.
    assert!(resp.symbols.iter().any(|s| s.name == "my_connect"));
}
