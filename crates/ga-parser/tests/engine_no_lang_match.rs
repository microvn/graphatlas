//! v1.1-M4 (S-005a D6) — AS-015 contract guard.
//!
//! "Adding a new language to graphatlas-v1.1 must NOT require changes to
//! the shared extraction engine."
//!
//! Concretely: engine modules (`calls.rs`, `references.rs`, `extends.rs`)
//! MUST NOT contain `match Lang::*` or `if matches!(lang, Lang::*)` control
//! flow. All per-lang dispatch is via `LanguageSpec` trait fn-pointer
//! tables (`callee_extractors()` / `ref_emitters()`).
//!
//! Pre-D3/D4: engine had 3 `matches!(lang, ...)` arms in references.rs +
//! 4 hardcoded per-kind branches in calls.rs (decorator/new/jsx/macro).
//! D3 + D4 migrated all of them into per-lang `langs/*.rs` files.
//!
//! Scope: D3/D4 modules only. `imports.rs` retains 3 lang-conditional
//! branches (pre-existing infra:S-002 work, NOT introduced by Phase A) —
//! tracked as carve-out for follow-up migration.

use std::fs;
use std::path::PathBuf;

/// Engine modules whose `match Lang::*` patterns the migration targeted.
const D3_D4_ENGINE_MODULES: &[&str] = &["src/calls.rs", "src/references.rs", "src/extends.rs"];

fn ga_parser_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Read a source file, strip line comments, return remaining lines.
/// Comment stripping is conservative: only lines whose first non-whitespace
/// char is `/` (start of `//` or `/*`) are removed. Inline trailing
/// comments survive — but the patterns we forbid would never appear there
/// in real code (always at start of a control-flow construct).
fn non_comment_lines(content: &str) -> Vec<(usize, &str)> {
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with("*")
        })
        .collect()
}

#[test]
fn d3_d4_engine_modules_have_no_match_lang_arms() {
    let root = ga_parser_root();
    let mut violations: Vec<String> = Vec::new();

    for relpath in D3_D4_ENGINE_MODULES {
        let path = root.join(relpath);
        let content = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relpath}: {e}"));

        for (lineno, line) in non_comment_lines(&content) {
            // Forbidden patterns:
            //  1. `match lang {` or `match self.lang {` etc.
            //  2. `matches!(lang, Lang::Foo ...)` or `matches!(self.lang, Lang::Foo ...)`
            //  3. `if let Lang::Foo = lang` (rare but should be caught)
            let forbidden = (line.contains("match ") && line.contains("lang"))
                || (line.contains("matches!(") && line.contains("Lang::"))
                || (line.contains("if let Lang::"));
            if forbidden {
                violations.push(format!("{relpath}:{}: {}", lineno + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "AS-015 contract violation — engine module(s) contain `match Lang::*` patterns:\n{}\n\nAll per-lang dispatch must go through LanguageSpec trait fn-pointer tables \
        (callee_extractors / ref_emitters). See langs/*.rs for the pattern.",
        violations.join("\n")
    );
}

#[test]
fn engine_no_lang_match_does_not_cover_imports_yet() {
    // Carve-out documentation: `imports.rs` still has 3 lang-conditional
    // branches (TS/JS export_statement re-export, Python imported_names,
    // TS/JS imported_names/aliases). These are pre-existing infra:S-002
    // work, NOT introduced by Phase A. Migration tracked for follow-up.
    //
    // This test exists to document the carve-out so the AS-015 guard
    // above is not silently extended without a deliberate decision.
    let path = ga_parser_root().join("src/imports.rs");
    let content = fs::read_to_string(&path).unwrap();
    let has_lang_match = content
        .lines()
        .any(|line| line.contains("match lang") || line.contains("matches!(lang, Lang::"));
    assert!(
        has_lang_match,
        "If imports.rs no longer has `match lang`, fold it into D3_D4_ENGINE_MODULES \
         in the test above and delete this carve-out test."
    );
}
