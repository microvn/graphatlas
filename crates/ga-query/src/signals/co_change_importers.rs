//! EXP-M2-11 — Phase C intersection (importers ∩ co-change ≥ threshold).
//!
//! Mirrors `scripts/extract-seeds.ts:491-538` Phase C that derives
//! `should_touch_files` offline. Re-computed at query time so impact()
//! returns structural blast radius that the graph BFS alone misses
//! (inheritance/interface impls that don't directly CALL seed).
//!
//! H-M6 stability audit (`bench-results/m2-h6-stability-audit.md`) confirmed
//! this variant passes S/N gate (2.04) with +18.9% `blast_radius_coverage`
//! lift on dev corpus. Threshold ≥3 chosen over ≥2 for tightest precision.

use super::co_change::{get_co_change_files, DEFAULT_MAX_COMMIT_SIZE, DEFAULT_N_COMMITS};
use super::importers::{git_grep_importers, import_grep_spec};
use std::collections::HashSet;
use std::path::Path;

/// Default threshold — files must co-change ≥3 times with seed_file
/// to qualify (after importer intersection). Matches H-M6 winning variant.
pub const DEFAULT_THRESHOLD: u32 = 3;

/// Docs / changelog / build exclusions borrowed from EXP-015. These
/// co-change heavily but add noise without blast-radius value.
fn is_excluded_noise(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".rst")
        || lower.ends_with(".txt")
        || lower.starts_with("docs/")
        || lower.contains("/docs/")
        || lower.starts_with("changelog")
        || lower.contains("/changelog")
}

/// Compute `importers(seed_file, lang) ∩ coChange(seed_file) ≥ threshold`.
///
/// Returns empty set when:
/// - `lang` not supported by [`import_grep_spec`]
/// - No importers found at HEAD
/// - Repo has no git history or grep unavailable
///
/// Seed file itself is excluded from the result.
pub fn compute_co_change_importers(
    repo: &Path,
    seed_file: &str,
    lang: &str,
    threshold: u32,
) -> HashSet<String> {
    let Some(spec) = import_grep_spec(repo, seed_file, lang) else {
        return HashSet::new();
    };
    let importers = git_grep_importers(repo, &spec);
    if importers.is_empty() {
        return HashSet::new();
    }
    let cc_map = get_co_change_files(repo, seed_file, DEFAULT_N_COMMITS, DEFAULT_MAX_COMMIT_SIZE);
    cc_map
        .into_iter()
        .filter(|(_, n)| *n >= threshold)
        .map(|(f, _)| f)
        .filter(|f| f != seed_file)
        .filter(|f| !is_excluded_noise(f))
        .filter(|f| importers.contains(f))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nongit_dir_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Write a stub go.mod to satisfy spec builder, but no git = no
        // importers + no co-change = empty result.
        std::fs::write(tmp.path().join("go.mod"), "module example.com\n").unwrap();
        let result = compute_co_change_importers(tmp.path(), "pkg/foo.go", "go", 3);
        assert!(result.is_empty(), "non-git dir must yield empty set");
    }

    #[test]
    fn unsupported_lang_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = compute_co_change_importers(tmp.path(), "foo.cobol", "cobol", 3);
        assert!(result.is_empty());
    }

    #[test]
    fn excluded_noise_pattern_matches_docs_md() {
        assert!(is_excluded_noise("docs/intro.md"));
        assert!(is_excluded_noise("CHANGELOG.md"));
        assert!(is_excluded_noise("api/docs/overview.rst"));
        assert!(!is_excluded_noise("src/handler.rs"));
        assert!(!is_excluded_noise("pkg/foo.go"));
    }
}
