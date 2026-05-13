//! EXP-M2-TEXTFILTER — post-BFS content intersect filter.
//!
//! Drops impacted files whose content doesn't mention the seed symbol as
//! a word-boundary token. Hub files reached via transitive CALLS +
//! REFERENCES (e.g. `django.utils.functional` touched by dozens of
//! seeds) rarely mention the specific seed textually — filtering them
//! out recovers precision without hurting completeness much.
//!
//! Validated on dev corpus 2026-04-24:
//! precision +0.205, completeness −0.043, signal_lost:noise_removed
//! = 1:318. Estimated composite delta +0.018.

use super::types::ImpactedFile;
use std::collections::HashSet;
use std::path::Path;

/// `true` when `seed` appears in `text` as a standalone identifier
/// (both boundaries are non-ident chars or string edges).
/// Empty seeds never match.
pub(super) fn contains_seed_token(text: &str, seed: &str) -> bool {
    if seed.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let sbytes = seed.as_bytes();
    if sbytes.len() > bytes.len() {
        return false;
    }
    let mut i = 0;
    while i + sbytes.len() <= bytes.len() {
        if &bytes[i..i + sbytes.len()] == sbytes {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_ok =
                i + sbytes.len() == bytes.len() || !is_ident_byte(bytes[i + sbytes.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Filter `files` to those whose content contains `seed` as a
/// word-boundary token. `depth == 0` (seed-defining files) always
/// survive as a safety net — generated code or unusual syntax might
/// hide the seed token even though the graph has the definition.
///
/// Unreadable files (missing, non-utf8) are dropped. On empty
/// `repo_root`, all non-depth-0 files are dropped conservatively —
/// matches the existing `routes::collect_affected_routes` behavior.
#[allow(dead_code)] // superseded by filter_by_path_symbols (multi-token, option b)
pub(super) fn filter_by_seed_text(
    files: Vec<ImpactedFile>,
    seed: &str,
    repo_root: &Path,
) -> Vec<ImpactedFile> {
    files
        .into_iter()
        .filter(|f| {
            if f.depth == 0 {
                return true;
            }
            let full = repo_root.join(&f.path);
            let Ok(bytes) = std::fs::read(&full) else {
                return false;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return false;
            };
            contains_seed_token(text, seed)
        })
        .collect()
}

/// Multi-token variant per AS-016 investigation 2026-04-25 option (b):
/// keep file when its text contains ANY identifier from the BFS path
/// symbol set as a word-boundary token. The set is whatever
/// `bfs::bfs_from_symbol` accumulates as `visited` — every symbol
/// expanded during the walk, including the seed.
///
/// Restores filter on the symbol-direct path (which option (a) had
/// disabled to keep AS-016 green) without sacrificing AS-016: the chain
/// `alpha ← beta ← gamma` puts {alpha, beta, gamma} in the set, and
/// c.py contains `beta` and `gamma` as tokens — survives.
///
/// `depth == 0` files always survive (seed-defining safety net for
/// generated code where the symbol token may be absent textually).
pub(super) fn filter_by_path_symbols(
    files: Vec<ImpactedFile>,
    symbols: &HashSet<String>,
    repo_root: &Path,
) -> Vec<ImpactedFile> {
    files
        .into_iter()
        .filter(|f| {
            if f.depth == 0 {
                return true;
            }
            let full = repo_root.join(&f.path);
            let Ok(bytes) = std::fs::read(&full) else {
                return false;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return false;
            };
            symbols.iter().any(|sym| contains_seed_token(text, sym))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_seed_token_basic_match() {
        assert!(contains_seed_token("def foo():", "foo"));
        assert!(contains_seed_token("call foo()", "foo"));
        assert!(contains_seed_token("foo = 1", "foo"));
    }

    #[test]
    fn contains_seed_token_word_boundary() {
        // Substring-only must NOT match.
        assert!(!contains_seed_token("footprint_size", "foo"));
        assert!(!contains_seed_token("def footprint():", "foo"));
        assert!(!contains_seed_token("superfoo = 1", "foo"));
    }

    #[test]
    fn contains_seed_token_handles_edges() {
        assert!(!contains_seed_token("", "foo"));
        assert!(!contains_seed_token("foo", ""));
        assert!(contains_seed_token("foo", "foo"));
        // seed longer than text
        assert!(!contains_seed_token("fo", "foo"));
    }

    #[test]
    fn contains_seed_token_various_delimiters() {
        for ch in [" ", "(", ")", ".", ",", "\n", "\t", ":", ";", "=", "["] {
            let text = format!("x{ch}foo{ch}y");
            assert!(
                contains_seed_token(&text, "foo"),
                "delim {:?} should work: text={:?}",
                ch,
                text
            );
        }
    }
}
