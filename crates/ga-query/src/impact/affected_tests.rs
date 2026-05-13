//! Cluster C4 — affected-tests discovery (TESTED_BY edges + convention match).

use super::types::{AffectedTest, AffectedTestReason};
use crate::common;
use crate::signals::co_change;
use ga_core::{Error, Result};
use ga_index::Store;
use std::collections::HashMap;
use std::path::Path;

/// Co-change threshold for the `affected_tests` signal — a candidate
/// test file must co-change with seed_file in at least this many recent
/// commits before it surfaces. Conservative; mockito M2 inspection
/// (2026-04-26) shows expected tests typically co-change 2-5 times.
const COCHANGE_MIN_COMMITS: u32 = 2;

/// Tests that cover the seed symbol — union of TESTED_BY edges (currently
/// empty because the indexer doesn't emit them — see TODO.md) and
/// convention-matched files whose path mentions the seed or its file stem.
///
/// Convention patterns:
/// - Python: `test_*.py`, `*_test.py`, `tests/`, `test/` segments
/// - TS/JS:  `*.test.{ts,tsx,js,jsx,mjs,cjs}`, `*.spec.*`, `__tests__/` segments
/// - Go:     `*_test.go`
/// - Rust:   `tests/*.rs`, `*_test.rs`
///
/// Relevance: the test path (case-sensitive) contains the seed symbol name
/// OR one of the seed file stems. This keeps downstream noise low when a
/// repo has many unrelated test files.
pub(super) fn collect_affected_tests(
    store: &Store,
    seed_symbol: &str,
    seed_stems: &[String],
    repo_root: &Path,
) -> Result<Vec<AffectedTest>> {
    if !common::is_safe_ident(seed_symbol) {
        return Ok(Vec::new());
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    let mut by_path: HashMap<String, AffectedTestReason> = HashMap::new();

    // (1a) Direct TESTED_BY edge — seed itself is the production symbol.
    let cypher = format!(
        "MATCH (prod:Symbol)-[:TESTED_BY]->(test:Symbol) \
         WHERE prod.name = '{seed_symbol}' RETURN DISTINCT test.file"
    );
    if let Ok(rs) = conn.query(&cypher) {
        for row in rs {
            if let Some(lbug::Value::String(path)) = row.into_iter().next() {
                by_path.insert(path, AffectedTestReason::Edge);
            }
        }
    }

    // (1b) EXP-M2-05 — transitive CALLS*1..3 chain before TESTED_BY.
    // Covers production symbols reachable from the seed within 3 directed
    // CALLS hops; captures tests of downstream helpers/utilities the seed
    // depends on. Direction matches the TESTED_BY emission convention
    // (prod -[TESTED_BY]-> test, indexer.rs:287-325).
    let cypher = format!(
        "MATCH (seed:Symbol)-[:CALLS*1..3]->(prod:Symbol)-[:TESTED_BY]->(test:Symbol) \
         WHERE seed.name = '{seed_symbol}' RETURN DISTINCT test.file"
    );
    if let Ok(rs) = conn.query(&cypher) {
        for row in rs {
            if let Some(lbug::Value::String(path)) = row.into_iter().next() {
                by_path.entry(path).or_insert(AffectedTestReason::Edge);
            }
        }
    }

    // KG-10: When seed itself lives in a test file, include seed's own file.
    // The TESTED_BY edges point FROM production, so tests are never prod —
    // phase (1a) skips them. The transitive chain (1b) already handles any
    // CALLS from the seed, so we only need to add the seed's own file here.
    let seed_files = query_symbol_files(&conn, seed_symbol);
    let seed_is_test = seed_files.iter().any(|f| common::is_test_path(f));
    if seed_is_test {
        for sf in &seed_files {
            if common::is_test_path(sf) {
                by_path
                    .entry(sf.clone())
                    .or_insert(AffectedTestReason::Edge);
            }
        }
    }

    // (2) Import-aware test detection — test files that IMPORT the seed
    //     symbol's defining file. Catches Java/Kotlin/C# cases where a
    //     test imports a class but emits no CALLS edge (e.g., the test
    //     uses the class as a mock target, reflection token, or generic
    //     type parameter — patterns mockito's reflection-heavy test suite
    //     uses heavily). Pre-this-phase ripgrep beat GA on Java
    //     test_recall (0.389 vs 0.000 → 0.222) precisely because text
    //     grep saw `import org.mockito.X` while the graph chain didn't
    //     walk through it. This phase makes GA structurally equivalent.
    //
    //     Query: test_file -[IMPORTS]-> prod_file -[DEFINES]-> seed_symbol
    //     The IMPORTS edge is File→File (per indexer schema); DEFINES is
    //     File→Symbol. seed_symbol is `is_safe_ident`-validated above so
    //     direct interpolation is safe (matches the policy used by
    //     TESTED_BY queries earlier in this fn).
    let cypher = format!(
        "MATCH (t:File)-[:IMPORTS]->(prod:File)-[:DEFINES]->(s:Symbol) \
         WHERE s.name = '{seed_symbol}' RETURN DISTINCT t.path"
    );
    if let Ok(rs) = conn.query(&cypher) {
        for row in rs {
            let Some(lbug::Value::String(path)) = row.into_iter().next() else {
                continue;
            };
            if !common::is_test_path(&path) {
                continue;
            }
            by_path
                .entry(path)
                .or_insert(AffectedTestReason::Convention);
        }
    }

    // (3) Convention match over all indexed files.
    //     - Direct: path mentions seed_symbol or seed_stem.
    //     - Co-package (Java/Kotlin Maven layout): seed_file lives in
    //       src/main/<lang>/<pkg>/...; surface test files under
    //       src/test/<lang>/<pkg>/ even when path_mentions misses.
    //       Empirical justification (mockito M2 inspection 2026-04-26):
    //       seed=`InlineBytecodeGenerator` expects test
    //       `InlineDelegateByteBuddyMockMakerTest.java` in the same
    //       package — different stem, no path_mentions hit, no CALLS
    //       chain (reflection-based mocking). Co-package convention
    //       captures these cases without needing IMPORTS resolution
    //       (which doesn't work for Java FQN→path mapping at present).
    let co_package_dirs: Vec<String> = seed_files
        .iter()
        .flat_map(|f| co_package_test_dirs(f))
        .collect();

    let rs = conn
        .query("MATCH (f:File) RETURN f.path")
        .map_err(|e| Error::Other(anyhow::anyhow!("file-scan query: {e}")))?;
    for row in rs {
        let Some(lbug::Value::String(path)) = row.into_iter().next() else {
            continue;
        };
        if !common::is_test_path(&path) {
            continue;
        }
        let mention_hit = path_mentions(&path, seed_symbol, seed_stems);
        let copkg_hit = co_package_dirs.iter().any(|d| path.starts_with(d.as_str()));
        if !mention_hit && !copkg_hit {
            continue;
        }
        // Edge reason wins over convention when both match the same file.
        by_path
            .entry(path)
            .or_insert(AffectedTestReason::Convention);
    }

    // (4) Co-change signal — files that co-change with the seed file in
    //     recent git history. Catches GT mining tasks (mockito-style)
    //     where expected tests have NO structural edge to the seed but
    //     consistently change in the same fix commits. Mirrors the
    //     should_touch_files Phase B derivation in extract-seeds.ts.
    //     Skipped silently when repo_root is empty (M0 callers without
    //     git context) or when the dir isn't a git checkout.
    if !repo_root.as_os_str().is_empty() {
        for sf in &seed_files {
            let map = co_change::get_co_change_files(
                repo_root,
                sf,
                co_change::DEFAULT_N_COMMITS,
                co_change::DEFAULT_MAX_COMMIT_SIZE,
            );
            for (path, count) in map {
                if count < COCHANGE_MIN_COMMITS {
                    continue;
                }
                if !common::is_test_path(&path) {
                    continue;
                }
                by_path
                    .entry(path)
                    .or_insert(AffectedTestReason::Convention);
            }
        }
    }

    // (5) Text-fallback — only fires when phases 1-4 produced nothing.
    //     Reads test files on disk and surfaces those whose content
    //     mentions seed_symbol as a word-boundary token. Steals the
    //     ripgrep advantage on tests that reference the seed only via
    //     comment / string literal / mock target token (mockito
    //     reflection patterns). Conservative: gated on by_path being
    //     empty so it cannot inflate noise on the precision dim when
    //     the structural signals already work.
    if by_path.is_empty() && !repo_root.as_os_str().is_empty() {
        let rs = conn
            .query("MATCH (f:File) RETURN f.path")
            .map_err(|e| Error::Other(anyhow::anyhow!("file-scan query: {e}")))?;
        for row in rs {
            let Some(lbug::Value::String(path)) = row.into_iter().next() else {
                continue;
            };
            if !common::is_test_path(&path) {
                continue;
            }
            if file_contains_token(repo_root, &path, seed_symbol) {
                by_path
                    .entry(path)
                    .or_insert(AffectedTestReason::Convention);
            }
        }
    }

    let mut out: Vec<AffectedTest> = by_path
        .into_iter()
        .map(|(path, reason)| AffectedTest { path, reason })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Word-boundary check on file content. Returns false on read errors
/// (missing file, non-utf8) — same conservative policy as
/// `text_filter::filter_by_seed_text`.
fn file_contains_token(repo_root: &Path, rel: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let full = repo_root.join(rel);
    let Ok(bytes) = std::fs::read(&full) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    contains_word_boundary(text, token)
}

fn contains_word_boundary(haystack: &str, token: &str) -> bool {
    let bytes = haystack.as_bytes();
    let needle = token.as_bytes();
    let n = needle.len();
    if n == 0 || bytes.len() < n {
        return false;
    }
    for i in 0..=bytes.len() - n {
        if &bytes[i..i + n] != needle {
            continue;
        }
        let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
        let after_ok = i + n == bytes.len() || !is_ident_byte(bytes[i + n]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Files that define a symbol with the given name. Used by KG-10 to detect
/// whether the seed itself lives in a test file.
fn query_symbol_files(conn: &lbug::Connection<'_>, name: &str) -> Vec<String> {
    let cypher = format!("MATCH (s:Symbol) WHERE s.name = '{name}' RETURN DISTINCT s.file");
    let Ok(rs) = conn.query(&cypher) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(f)) = row.into_iter().next() {
            out.push(f);
        }
    }
    out
}

/// Maven/Gradle co-package mirror directories where Java/Kotlin tests
/// live. `src/main/<lang>/<pkg>/<File>.<ext>` →
/// `src/test/<lang>/<pkg>/` (the directory, with trailing slash so
/// `starts_with` checks are unambiguous against prefix collisions).
///
/// Returns 0..N entries:
/// - 0 if seed_file isn't in a Maven main layout (Python/Go/Rust/JS/TS).
/// - 1 for a single Maven module.
/// - >1 for nested-pkg edge cases (currently never emitted; reserved).
fn co_package_test_dirs(seed_file: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (main_marker, test_marker) in &[
        ("/src/main/java/", "/src/test/java/"),
        ("/src/main/kotlin/", "/src/test/kotlin/"),
    ] {
        if let Some(idx) = seed_file.find(main_marker) {
            let prefix = &seed_file[..idx];
            let after = &seed_file[idx + main_marker.len()..];
            let pkg_dir = after.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            if pkg_dir.is_empty() {
                continue;
            }
            out.push(format!("{prefix}{test_marker}{pkg_dir}/"));
        }
    }
    // Top-level Maven module (no leading prefix).
    for (main_marker, test_marker) in &[
        ("src/main/java/", "src/test/java/"),
        ("src/main/kotlin/", "src/test/kotlin/"),
    ] {
        if let Some(after) = seed_file.strip_prefix(*main_marker) {
            let pkg_dir = after.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            if !pkg_dir.is_empty() {
                let mirror = format!("{test_marker}{pkg_dir}/");
                if !out.contains(&mirror) {
                    out.push(mirror);
                }
            }
        }
    }
    out
}

/// `true` when `path` mentions the seed symbol or one of the seed file stems.
/// Case-sensitive substring match — cheap and language-agnostic.
fn path_mentions(path: &str, symbol: &str, stems: &[String]) -> bool {
    if path.contains(symbol) {
        return true;
    }
    stems
        .iter()
        .any(|stem| !stem.is_empty() && path.contains(stem))
}
