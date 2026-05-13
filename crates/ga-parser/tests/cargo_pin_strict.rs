//! v1.1-M4 (S-005a D6) — AS-016: tree-sitter grammar SHA pinning regression.
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-016):
//!   "Cargo.toml pins tree-sitter-java = X.Y.Z + CI test runs fixture AST
//!    snapshot. When tree-sitter grammar bumps major → CI fails the
//!    fixture snapshot test."
//!
//! Two-layer guarantee:
//!  1. `grammar_drift.rs` (existing) — RUNTIME: parses fixture per lang,
//!     asserts every kind in `*_node_kinds()` const lists is still emitted.
//!     Catches semantic drift even within a "patch" version bump.
//!  2. THIS test — STATIC: greps Cargo.toml, asserts every tree-sitter-*
//!     dep declares a version pin (no `*`, no `>=`). Catches accidental
//!     pin loosening at PR review time, before anyone runs tests.

use std::fs;
use std::path::PathBuf;

fn cargo_toml_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

#[test]
fn all_tree_sitter_deps_declare_a_version_pin() {
    let content = fs::read_to_string(cargo_toml_path()).unwrap();

    let mut violations: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (lineno, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Skip comments / blanks / non-dependency lines.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Match `tree-sitter` and `tree-sitter-*` dependency declarations.
        // Format examples:
        //   tree-sitter = "0.25"
        //   tree-sitter-python = "0.23"
        //   tree-sitter-foo = { version = "0.23", features = [...] }
        if !trimmed.starts_with("tree-sitter") || !trimmed.contains('=') {
            continue;
        }
        // Parse `<dep-name> =` head.
        let eq_pos = trimmed.find('=').unwrap();
        let dep_name = trimmed[..eq_pos].trim().to_string();
        // Skip non-tree-sitter false positives (e.g., comments matching).
        if !dep_name.starts_with("tree-sitter") {
            continue;
        }
        seen.push(dep_name.clone());

        let value = trimmed[eq_pos + 1..].trim();

        // Reject wildcard version: `tree-sitter-x = "*"` or `version = "*"`.
        if value.contains("\"*\"") {
            violations.push(format!(
                "{}:{}: {} uses wildcard \"*\" — must pin a version",
                "Cargo.toml",
                lineno + 1,
                dep_name
            ));
            continue;
        }
        // Reject open-ended `>=` requirements: `tree-sitter-x = ">=0.23"`.
        if value.contains("\">=") {
            violations.push(format!(
                "{}:{}: {} uses open `>=` — must pin a major.minor at minimum",
                "Cargo.toml",
                lineno + 1,
                dep_name
            ));
            continue;
        }
        // Reject missing version (table without `version = "..."`).
        // Inline tables containing only `path = ...` or `git = ...` would
        // bypass crates.io pinning entirely.
        if value.starts_with('{') && !value.contains("version") {
            violations.push(format!(
                "{}:{}: {} inline table missing `version` key — must pin",
                "Cargo.toml",
                lineno + 1,
                dep_name
            ));
        }
    }

    assert!(
        !seen.is_empty(),
        "AS-016: no tree-sitter deps detected in Cargo.toml — test is \
         not exercising real input. Check ga-parser/Cargo.toml format."
    );
    assert!(
        violations.is_empty(),
        "AS-016 grammar pin violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn cargo_toml_includes_all_v1_grammars() {
    // Sanity: at least the 5 v1 grammars must be present. v1.1-M4 lang
    // stories (S-001..S-004) add Java/Kotlin/CSharp/Ruby grammars later.
    let content = fs::read_to_string(cargo_toml_path()).unwrap();
    for required in &[
        "tree-sitter-python",
        "tree-sitter-typescript",
        "tree-sitter-javascript",
        "tree-sitter-go",
        "tree-sitter-rust",
    ] {
        assert!(
            content.contains(required),
            "AS-016 prereq: ga-parser/Cargo.toml missing {required}"
        );
    }
}

#[test]
fn cargo_toml_includes_java_grammar() {
    // v1.1-M4 S-001a — `tree-sitter-java` becomes a required grammar dep
    // when JavaLang ships. Removing the dep without removing the spec impl
    // would break compilation, but a regression guard here catches the
    // intent of "Java is a v1.1-supported lang" at PR review time.
    let content = fs::read_to_string(cargo_toml_path()).unwrap();
    assert!(
        content.contains("tree-sitter-java"),
        "v1.1-M4 S-001 prereq: ga-parser/Cargo.toml must declare tree-sitter-java"
    );
}

#[test]
fn cargo_toml_includes_ruby_grammar() {
    // v1.1-M4 S-004a — `tree-sitter-ruby` becomes a required grammar dep
    // when RubyLang ships. Mirror of the Java assertion: declares the
    // intent of "Ruby is a v1.1-supported lang" at PR review time.
    let content = fs::read_to_string(cargo_toml_path()).unwrap();
    assert!(
        content.contains("tree-sitter-ruby"),
        "v1.1-M4 S-004 prereq: ga-parser/Cargo.toml must declare tree-sitter-ruby"
    );
}
