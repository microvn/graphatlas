//! Cluster C9 — git diff input parsing. Extracts repo-relative file paths
//! from unified-diff headers (`--- a/X` / `+++ b/X`) so the rest of the
//! pipeline can treat a diff like `changed_files` (C8 union semantics).
//!
//! Not a full diff parser — we only care about the *set of touched paths*,
//! not hunks or content. The grammar consumed:
//!
//! ```text
//! --- a/<old-path>
//! +++ b/<new-path>
//! ```
//!
//! `/dev/null` on either side means add / delete; we fall back to the
//! non-null side. Both `a/` and `b/` prefixes are stripped; trailing
//! tab-separated timestamp metadata is dropped.

use std::collections::HashSet;

/// Scan a unified diff and return the set of touched paths, deduplicated in
/// first-seen order. Header lines that don't contain a recognizable path
/// (or point only to `/dev/null`) are skipped.
pub(super) fn extract_files_from_diff(diff: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut files: Vec<String> = Vec::new();
    for line in diff.lines() {
        let tag = if let Some(rest) = line.strip_prefix("+++ ") {
            rest
        } else if let Some(rest) = line.strip_prefix("--- ") {
            rest
        } else {
            continue;
        };
        if let Some(path) = parse_diff_path(tag) {
            if seen.insert(path.clone()) {
                files.push(path);
            }
        }
    }
    files
}

fn parse_diff_path(raw: &str) -> Option<String> {
    // Strip trailing tab + timestamp — "path\t2024-01-01 12:00:00" pattern.
    let raw = raw.split('\t').next()?.trim();
    if raw.is_empty() || raw == "/dev/null" {
        return None;
    }
    // Git diff headers use "a/" / "b/" prefixes by convention.
    let trimmed = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_modified_file() {
        let diff = "--- a/src/foo.py\n\
                    +++ b/src/foo.py\n\
                    @@ -1 +1 @@\n\
                    -x\n+y\n";
        assert_eq!(
            extract_files_from_diff(diff),
            vec!["src/foo.py".to_string()]
        );
    }

    #[test]
    fn extracts_multiple_files() {
        let diff = "--- a/a.py\n+++ b/a.py\n\
                    --- a/b.py\n+++ b/b.py\n\
                    --- a/c.py\n+++ b/c.py\n";
        assert_eq!(
            extract_files_from_diff(diff),
            vec!["a.py".to_string(), "b.py".to_string(), "c.py".to_string()]
        );
    }

    #[test]
    fn new_file_uses_plus_side_when_old_is_dev_null() {
        let diff = "--- /dev/null\n+++ b/new_file.py\n@@\n+content\n";
        assert_eq!(
            extract_files_from_diff(diff),
            vec!["new_file.py".to_string()]
        );
    }

    #[test]
    fn deleted_file_uses_minus_side_when_new_is_dev_null() {
        let diff = "--- a/removed.py\n+++ /dev/null\n@@\n-content\n";
        assert_eq!(
            extract_files_from_diff(diff),
            vec!["removed.py".to_string()]
        );
    }

    #[test]
    fn strips_a_and_b_prefixes() {
        let diff = "--- a/path/one.py\n+++ b/path/two.py\n";
        let files = extract_files_from_diff(diff);
        assert!(files.contains(&"path/one.py".to_string()));
        assert!(files.contains(&"path/two.py".to_string()));
    }

    #[test]
    fn dedupes_paths_appearing_in_both_sides() {
        // `a/x` and `b/x` collapse to the same path after prefix stripping.
        let diff = "--- a/x.py\n+++ b/x.py\n";
        assert_eq!(extract_files_from_diff(diff), vec!["x.py".to_string()]);
    }

    #[test]
    fn trims_trailing_timestamp_metadata() {
        let diff = "--- a/foo.py\t2024-01-01 12:00:00 +0000\n\
                    +++ b/foo.py\t2024-01-02 12:00:00 +0000\n";
        assert_eq!(extract_files_from_diff(diff), vec!["foo.py".to_string()]);
    }

    #[test]
    fn empty_diff_yields_empty_vec() {
        assert!(extract_files_from_diff("").is_empty());
    }

    #[test]
    fn diff_without_headers_yields_empty_vec() {
        let diff = "this is not a diff\njust some text\n";
        assert!(extract_files_from_diff(diff).is_empty());
    }

    #[test]
    fn git_format_patch_with_index_line_still_parsed() {
        let diff = "diff --git a/foo.py b/foo.py\n\
                    index abc..def 100644\n\
                    --- a/foo.py\n+++ b/foo.py\n@@\n";
        assert_eq!(extract_files_from_diff(diff), vec!["foo.py".to_string()]);
    }
}
