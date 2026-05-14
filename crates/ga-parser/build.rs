//! Build-time enforcement for graphatlas-v1.2-grammar-pins.md S-001 AS-002.
//!
//! Asserts that every `pub mod <lang>;` registered in `src/langs/mod.rs` has
//! a matching `[pins.<key>]` entry in `grammar-pins.toml`. Mirrors the
//! test-time check in `tests/grammar_pins_coverage.rs` (orthogonal visibility
//! per spec — compile fails fast on missing pins, `cargo test` catches the
//! same drift if a dev skips the build path).
//!
//! Intentionally NO serde/toml dependencies — uses simple line parsing to
//! avoid extending the build-dependency surface. The full schema check lives
//! in the test crate.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"),
    );

    let pins_path = manifest_dir.join("grammar-pins.toml");
    let mod_rs = manifest_dir.join("src/langs/mod.rs");

    // Re-run only when these files change.
    println!("cargo:rerun-if-changed=grammar-pins.toml");
    println!("cargo:rerun-if-changed=src/langs/mod.rs");

    // Parse `pub mod <name>;` lines from mod.rs, ignoring `shared`.
    let registered = parse_mod_rs(&mod_rs);

    // Parse `[pins.<key>]` headers from grammar-pins.toml.
    let pin_keys = parse_pin_keys(&pins_path);

    // Apply the same mod→key shorthand the test uses.
    let mut missing = Vec::new();
    for lang in &registered {
        let key = mod_to_pin_key(lang);
        if !pin_keys.contains(key) {
            missing.push(format!("mod {lang} → expected [pins.{key}]"));
        }
    }

    if !missing.is_empty() {
        for line in &missing {
            println!("cargo:warning=grammar-pins.toml missing entry: {line}");
        }
        panic!(
            "grammar-pins.toml is missing entries for {} registered lang(s) — see warnings above. \
             Spec: docs/specs/graphatlas-v1.2/graphatlas-v1.2-grammar-pins.md S-001 AS-002.",
            missing.len()
        );
    }
}

fn parse_mod_rs(path: &PathBuf) -> BTreeSet<String> {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            let name = rest.trim_end_matches(';').trim();
            if name != "shared" && !name.is_empty() {
                out.insert(name.to_string());
            }
        }
    }
    out
}

fn parse_pin_keys(path: &PathBuf) -> BTreeSet<String> {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("[pins.") {
            if let Some(key) = rest.strip_suffix(']') {
                out.insert(key.to_string());
            }
        }
    }
    out
}

fn mod_to_pin_key(mod_name: &str) -> &str {
    match mod_name {
        "py" => "python",
        "ts" => "typescript",
        "js" => "javascript",
        "rs" => "rust",
        "kotlin" => "kotlin-ng",
        other => other,
    }
}
