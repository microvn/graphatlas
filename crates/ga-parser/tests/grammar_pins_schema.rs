//! S-001 AS-001 — grammar-pins.toml schema verification.
//!
//! Asserts the file exists at `crates/ga-parser/grammar-pins.toml` and every
//! `[pins.<lang>]` entry carries the required fields with the expected shape:
//! - `upstream` non-empty `org/repo` form
//! - `commit` exactly 40 lowercase hex chars (full SHA, not short)
//! - `crate` non-empty (the crates.io name)
//! - `crate_version` non-empty semver-shaped
//!
//! Per spec: docs/specs/graphatlas-v1.2/graphatlas-v1.2-grammar-pins.md S-001.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, serde::Deserialize)]
struct PinFile {
    pins: BTreeMap<String, PinEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct PinEntry {
    upstream: String,
    commit: String,
    #[serde(rename = "crate")]
    krate: String,
    crate_version: String,
}

fn pins_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("grammar-pins.toml")
}

fn load_pins() -> PinFile {
    let raw = fs::read_to_string(pins_path()).expect(
        "grammar-pins.toml must exist at crates/ga-parser/grammar-pins.toml (S-001 AS-001)",
    );
    toml::from_str(&raw)
        .expect("grammar-pins.toml must parse as valid TOML with the PinFile schema")
}

#[test]
fn grammar_pins_toml_exists_and_parses() {
    let pins = load_pins();
    assert!(
        !pins.pins.is_empty(),
        "grammar-pins.toml must declare at least one [pins.<lang>] entry"
    );
}

#[test]
fn grammar_pins_has_required_entries() {
    // S-001 AS-001 + AS-004: 9 v1.1 langs + PHP = 10 entries.
    let pins = load_pins();
    let required = [
        "python",
        "typescript",
        "javascript",
        "go",
        "rust",
        "java",
        "kotlin-ng",
        "csharp",
        "ruby",
        "php",
    ];
    for lang in required {
        assert!(
            pins.pins.contains_key(lang),
            "grammar-pins.toml missing [pins.{lang}] entry — required per AS-001/AS-004"
        );
    }
    assert!(
        pins.pins.len() >= required.len(),
        "expected at least {} pin entries, got {}",
        required.len(),
        pins.pins.len()
    );
}

#[test]
fn grammar_pins_commit_is_40_hex_chars() {
    // S-001 AS-001: SHA must be full 40-char (not short hash). Lowercase per git convention.
    let pins = load_pins();
    for (lang, entry) in &pins.pins {
        assert_eq!(
            entry.commit.len(),
            40,
            "[pins.{lang}].commit must be 40-char SHA, got {} chars",
            entry.commit.len()
        );
        assert!(
            entry
                .commit
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "[pins.{lang}].commit must be lowercase hex, got '{}'",
            entry.commit
        );
    }
}

#[test]
fn grammar_pins_upstream_is_org_slash_repo() {
    let pins = load_pins();
    for (lang, entry) in &pins.pins {
        let parts: Vec<&str> = entry.upstream.split('/').collect();
        assert_eq!(
            parts.len(),
            2,
            "[pins.{lang}].upstream must be 'org/repo' form, got '{}'",
            entry.upstream
        );
        assert!(
            !parts[0].is_empty() && !parts[1].is_empty(),
            "[pins.{lang}].upstream has empty org or repo: '{}'",
            entry.upstream
        );
    }
}

#[test]
fn grammar_pins_crate_and_version_nonempty() {
    let pins = load_pins();
    for (lang, entry) in &pins.pins {
        assert!(
            !entry.krate.is_empty(),
            "[pins.{lang}].crate must be non-empty"
        );
        assert!(
            !entry.crate_version.is_empty(),
            "[pins.{lang}].crate_version must be non-empty"
        );
        // Loose semver check: at least one digit
        assert!(
            entry.crate_version.chars().any(|c| c.is_ascii_digit()),
            "[pins.{lang}].crate_version must contain at least one digit: '{}'",
            entry.crate_version
        );
    }
}
