//! v1.2-php S-002 AS-010 — programmatic enumeration of pattern-defining
//! `is_test_path` / `TEST_PATTERN` sites.
//!
//! Per /mf-challenge Critical finding C-2 (Failure + Assumption + Scope
//! convergence): the original spec asserted "11 prod sites" by hand-count.
//! Live grep on the codebase shows many more, and the hand-list rots the
//! moment a new retriever / GT rule / mining stage adds another site.
//!
//! This audit programmatically discovers every file that contains a
//! **language-specific test-path pattern literal** (e.g., `_test.go`,
//! `Test.java`, `_spec.rb`) and asserts each ALSO recognises PHPUnit
//! suffix (`Test.php` / `Tests.php`). The set of sites is data, not config.
//!
//! Pattern-CONSUMING sites (files that just call `is_test_path()` and
//! delegate the matching) auto-benefit when the canonical resolver gets
//! PHP coverage. Pattern-DEFINING sites are the ones that need lock-step
//! updates.

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Lang-specific test-path patterns that mark a file as a "pattern-definer".
/// Each pattern is specific enough to NOT collide with Rust `#[test] fn test_*`
/// or `test_foo` symbol names — these are filename suffixes / regex fragments
/// that only appear in code performing test-path classification.
///
/// Notably `test_` alone is omitted (false positive: every Rust test function).
/// The Python prefix `test_*.py` is detected via the literal regex fragment
/// `test_[^/]+\.py` which only appears in pattern-defining sites.
const DEFINING_PATTERNS: &[&str] = &[
    "_test.go",
    "_test.rs",
    "Test.java",
    "Test.kt",
    "Tests.cs",
    "_spec.rb",
    "_test.rb",
    "test_[^/]+\\.py", // regex fragment for Python prefix — only in path-classifier code
    ".test.ts",
    ".spec.ts",
];

/// Markers that confirm a file recognises PHPUnit (`*Test.php` / `*Tests.php`).
/// Liberal — any of these substrings means "PHP is covered here".
const PHP_RECOGNITION_MARKERS: &[&str] = &[
    "Test.php",
    "Tests.php",
    "\\.php$", // regex form in JS-side scripts
    "(?i)test\\.php",
];

/// Roots to walk. Excludes `target/`, `node_modules/`, etc. via skip predicate.
const WALK_ROOTS: &[&str] = &["crates", "scripts", "benches"];

/// Skip predicate — these dirs / file types are irrelevant to the audit.
///
/// Notably skips `/tests/` and `/examples/` directories: those are CONSUMERS
/// (test fixture data, demo code) not pattern-DEFINING sites. They may
/// contain literal strings like `_test.go` as input to is_test_path
/// assertions but they don't classify paths themselves.
fn should_skip(path: &str) -> bool {
    path.contains("/target/")
        || path.contains("/node_modules/")
        || path.contains("/.git/")
        || path.contains("/fixtures/")
        || path.contains("/snapshots/")
        || path.contains("/tests/")       // test crates — consumers, not definers
        || path.contains("/examples/")    // example bins — demo code, not definers
        || path.ends_with(".md")
}

fn walk_rust_and_ts(root: &PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_recursive(root, &mut out);
    out
}

fn walk_recursive(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        if should_skip(&path_str) {
            continue;
        }
        if path.is_dir() {
            walk_recursive(&path, out);
        } else if let Some(ext) = path.extension() {
            if matches!(ext.to_str(), Some("rs") | Some("ts") | Some("js")) {
                out.push(path);
            }
        }
    }
}

/// Strip line comments from Rust/TS source so the audit doesn't false-positive
/// on patterns mentioned in docstrings (`/// Go: *_test.go`) or inline
/// comments (`// see _spec.rb convention`). Comments describe patterns,
/// they don't classify paths.
fn strip_comments(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        // Drop the entire line if it starts with a Rust/JS comment marker.
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
            out.push('\n');
            continue;
        }
        // Drop trailing `//` line comments — keep code before them.
        if let Some(idx) = line.find("//") {
            // Be lenient about `//` inside strings — conservative approach:
            // only strip if the `//` is preceded by whitespace. Avoids
            // mangling URL literals like `"https://..."` in surface code.
            let before = &line[..idx];
            if before.chars().rev().take_while(|c| *c != '"').count() == before.len()
                && before
                    .chars()
                    .rev()
                    .find(|c| !c.is_whitespace())
                    .map(|c| !"\\".contains(c))
                    .unwrap_or(true)
            {
                out.push_str(before);
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn every_definer_recognises_phpunit_pattern() {
    let root = workspace_root();
    let mut all_files = Vec::new();
    for r in WALK_ROOTS {
        all_files.extend(walk_rust_and_ts(&root.join(r)));
    }

    // For each file, check if it contains ANY defining pattern in RUNTIME
    // code (strip comments first — they describe patterns, don't classify).
    let mut violations: Vec<String> = Vec::new();
    let mut checked = 0;
    for file in &all_files {
        let Ok(raw) = fs::read_to_string(file) else {
            continue;
        };
        let content = strip_comments(&raw);

        // Does this file define lang-specific test patterns?
        let has_definer = DEFINING_PATTERNS.iter().any(|p| content.contains(p));
        if !has_definer {
            continue;
        }
        checked += 1;

        // Does it ALSO recognise PHPUnit?
        let has_php = PHP_RECOGNITION_MARKERS.iter().any(|p| content.contains(p));
        if !has_php {
            let rel = file
                .strip_prefix(&root)
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| file.display().to_string());
            // Identify which definer triggered.
            let definers_present: Vec<&&str> = DEFINING_PATTERNS
                .iter()
                .filter(|p| content.contains(**p))
                .collect();
            violations.push(format!(
                "{rel} — defines lang-specific patterns {:?} but lacks PHPUnit recognition (Test.php / Tests.php)",
                definers_present
            ));
        }
    }

    assert!(
        checked > 0,
        "audit found ZERO definer sites — DEFINING_PATTERNS may be stale; manual inspection needed"
    );
    assert!(
        violations.is_empty(),
        "AS-010 lock-step violation — {} site(s) define lang-specific test patterns but miss PHPUnit coverage:\n  {}\n\n\
         Each violation is a graph-bias source: those sites will classify .php files inconsistently \
         with the canonical is_test_path, inflating or deflating competitor scores.",
        violations.len(),
        violations.join("\n  ")
    );

    eprintln!(
        "AS-010 audit OK — {} definer sites all recognise PHPUnit pattern",
        checked
    );
}

#[test]
fn audit_detects_canonical_is_test_path_site() {
    // Sanity: the canonical is_test_path in ga-query/src/common.rs MUST
    // be picked up by the audit (otherwise the audit's discovery is broken).
    let root = workspace_root();
    let canonical = root.join("crates/ga-query/src/common.rs");
    let content = fs::read_to_string(&canonical).expect("canonical is_test_path file must exist");
    let has_definer = DEFINING_PATTERNS.iter().any(|p| content.contains(p));
    assert!(
        has_definer,
        "canonical is_test_path file must contain ≥1 lang-specific test-path pattern — audit's discovery would miss it otherwise"
    );
}

#[test]
fn audit_finds_at_least_three_definer_sites() {
    // Sanity floor: if the audit only finds 1 site, DEFINING_PATTERNS is
    // probably too narrow. Currently we expect to find at least the canonical
    // is_test_path plus the TS scripts (mine-fix-commits.ts + extract-seeds.ts).
    let root = workspace_root();
    let mut all_files = Vec::new();
    for r in WALK_ROOTS {
        all_files.extend(walk_rust_and_ts(&root.join(r)));
    }
    let mut definer_count = 0;
    for file in &all_files {
        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };
        if DEFINING_PATTERNS.iter().any(|p| content.contains(p)) {
            definer_count += 1;
        }
    }
    assert!(
        definer_count >= 3,
        "audit only found {definer_count} definer sites — DEFINING_PATTERNS list is probably too narrow"
    );
}
