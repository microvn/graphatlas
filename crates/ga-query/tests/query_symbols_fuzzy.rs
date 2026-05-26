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

// P2.2 regression suite (2026-05-22) — substring-containing symbols must
// rank above symbols that share Levenshtein distance but don't contain the
// keyword. Real-world evidence: 4/5 audit rounds saw fuzzy return top-10
// symbols where 0/10 contained the queried keyword (e.g. pattern "middleware"
// returned `BindHeader`, `Handler`, `iterate` — all 0.95+ score).

#[test]
fn fuzzy_prefers_substring_match_over_equidistant_unrelated() {
    // Regression: P2.2 — symbols containing the pattern as substring should
    // outrank symbols that don't, even if both have similar Levenshtein
    // distance. Pre-fix: pure Levenshtein → equidistant unrelated symbols
    // rank as high as substring matches.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def MiddlewareChain(): pass\n\
         def UseMiddleware(): pass\n\
         def BindHeader(): pass\n\
         def Handler(): pass\n\
         def iterate(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "middleware", SymbolsMatch::Fuzzy).unwrap();
    assert!(!resp.symbols.is_empty());

    let names: Vec<&str> = resp.symbols.iter().map(|s| s.name.as_str()).collect();
    let pos_middleware_chain = names.iter().position(|&n| n == "MiddlewareChain");
    let pos_use_middleware = names.iter().position(|&n| n == "UseMiddleware");
    let pos_bind_header = names.iter().position(|&n| n == "BindHeader");

    assert!(
        pos_middleware_chain.is_some(),
        "MiddlewareChain must appear: {names:?}"
    );
    assert!(
        pos_use_middleware.is_some(),
        "UseMiddleware must appear: {names:?}"
    );
    // The actual regression: substring matches must outrank non-substring.
    if let (Some(p_chain), Some(p_bind)) = (pos_middleware_chain, pos_bind_header) {
        assert!(
            p_chain < p_bind,
            "MiddlewareChain must rank above BindHeader: {names:?}"
        );
    }
}

#[test]
fn fuzzy_score_separates_substring_from_non_substring() {
    // Regression: P2.2 — substring-match score must be visibly higher than
    // non-substring scores. Pre-fix scores were all ~0.95 (equidistant),
    // masking true matches.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def overflowBehavior(): pass\n\
         def error(): pass\n\
         def verify(): pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = symbols(&store, "overflow", SymbolsMatch::Fuzzy).unwrap();
    let overflow_score = resp
        .symbols
        .iter()
        .find(|s| s.name == "overflowBehavior")
        .map(|s| s.score)
        .unwrap_or(0.0);
    let non_match_max = resp
        .symbols
        .iter()
        .filter(|s| !s.name.to_lowercase().contains("overflow"))
        .map(|s| s.score)
        .fold(0.0f32, f32::max);
    assert!(
        overflow_score > non_match_max + 0.05,
        "substring match must score noticeably higher: \
         overflowBehavior={overflow_score}, max non-match={non_match_max}, all={:?}",
        resp.symbols
    );
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
