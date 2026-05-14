//! S-001 AS-002 — every lang registered in `src/langs/mod.rs` must have a
//! corresponding `[pins.<lang>]` entry in `grammar-pins.toml`.
//!
//! This test mirrors the build.rs check at test-time (orthogonal visibility
//! per spec AS-002). Build-time mismatch fails compile; test-time mismatch
//! fails `cargo test` — both fences guard the same invariant.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn ga_parser_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parse `src/langs/mod.rs` for `pub mod <name>;` entries. Ignores `shared`
/// (helper module, no LanguageSpec impl).
fn registered_langs() -> BTreeSet<String> {
    let mod_rs = ga_parser_root().join("src/langs/mod.rs");
    let src = fs::read_to_string(&mod_rs)
        .unwrap_or_else(|e| panic!("must read {}: {e}", mod_rs.display()));
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            let name = rest.trim_end_matches(';').trim();
            if name != "shared" {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Map a `mod` name (e.g. `py`, `csharp`) to the canonical `grammar-pins.toml`
/// key. Some modules use shorthand; pins use the crate-derived name.
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

fn pin_keys() -> BTreeSet<String> {
    let path = ga_parser_root().join("grammar-pins.toml");
    let raw =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("grammar-pins.toml must exist: {e}"));
    let parsed: toml::Value = toml::from_str(&raw).expect("grammar-pins.toml must parse as TOML");
    let pins_table = parsed
        .get("pins")
        .and_then(|v| v.as_table())
        .expect("grammar-pins.toml must have [pins] table");
    pins_table.keys().cloned().collect()
}

#[test]
fn every_registered_lang_has_pin_entry() {
    let registered = registered_langs();
    let pins = pin_keys();

    let mut missing = Vec::new();
    for lang in &registered {
        let key = mod_to_pin_key(lang);
        if !pins.contains(key) {
            missing.push(format!("mod {lang} → pins.{key} (not found)"));
        }
    }

    assert!(
        missing.is_empty(),
        "grammar-pins.toml missing entries for registered langs:\n  {}\n\n\
         Registered in src/langs/mod.rs: {:?}\n\
         Pin keys: {:?}",
        missing.join("\n  "),
        registered,
        pins,
    );
}

/// Pin entries that are intentionally forward-staged ahead of `pub mod <lang>;`
/// registration. Each entry needs a tracking spec/story so the staging is
/// auditable. Drop entries from this list once the corresponding lang adapter
/// ships in `src/langs/mod.rs`.
///
/// Format: `(pin-key, "<spec-file>.md <story-id>")` — the second element MUST
/// contain a story-id pattern `S-\d{3}` per
/// graphatlas-v1.2-grammar-pins.md S-001 AS-007.
const FORWARD_STAGED_PINS: &[(&str, &str)] = &[
    // Empty: PHP entry removed when v1.2-php.md S-001 registered `pub mod php;`
    // in src/langs/mod.rs. Add new rows here only when a future lang pin lands
    // ahead of its adapter — paired with `<spec-file>.md S-NNN`.
];

/// Returns true iff `s` contains a substring of the form `S-NNN` where NNN is
/// exactly three ASCII digits. No regex dependency — manual scan.
///
/// AS-007 wiring: every `FORWARD_STAGED_PINS` second element must satisfy
/// this so future readers can grep the spec for the story.
fn has_story_id_pattern(s: &str) -> bool {
    let bytes = s.as_bytes();
    // Need at least 5 chars: "S-" + 3 digits.
    if bytes.len() < 5 {
        return false;
    }
    for i in 0..=bytes.len() - 5 {
        if bytes[i] == b'S' && bytes[i + 1] == b'-' {
            let d1 = bytes[i + 2];
            let d2 = bytes[i + 3];
            let d3 = bytes[i + 4];
            if d1.is_ascii_digit() && d2.is_ascii_digit() && d3.is_ascii_digit() {
                // Optional 4th-digit guard: the spec says "exactly 3 digits". If a 4th
                // digit follows, it's a 4-digit ID — still acceptable as "contains" the
                // 3-digit prefix? AS-007 says `S-\d{3}` — `\d{3}` matches first 3 digits
                // regardless of what follows. So return true here.
                return true;
            }
        }
    }
    false
}

#[test]
fn has_story_id_pattern_recognizes_valid_ids() {
    // Positive cases — accepts canonical story-id format.
    assert!(has_story_id_pattern("graphatlas-v1.2-php.md S-001"));
    assert!(has_story_id_pattern("S-007"));
    assert!(has_story_id_pattern("foo S-123 bar"));
    assert!(has_story_id_pattern("M1 S-042 something"));
    assert!(has_story_id_pattern("S-9999")); // 4-digit IDs OK — first 3 satisfy \d{3}
}

#[test]
fn has_story_id_pattern_rejects_invalid_ids() {
    // Negative cases — guards against drift from the canonical format.
    assert!(!has_story_id_pattern(""));
    assert!(!has_story_id_pattern("S-")); // too short
    assert!(!has_story_id_pattern("S-01")); // only 2 digits
    assert!(!has_story_id_pattern("s-001")); // lowercase
    assert!(!has_story_id_pattern("S001")); // missing dash
    assert!(!has_story_id_pattern("future work TODO")); // free-form text
    assert!(!has_story_id_pattern("see later")); // free-form text
    assert!(!has_story_id_pattern("S-abc"));
}

#[test]
fn forward_staged_pins_reference_valid_story_ids() {
    // AS-007: every forward-staged entry's second element MUST contain a
    // `S-\d{3}` story-id pattern. Prevents free-form / vague destinations like
    // "future work" or "TODO.md" sneaking in.
    let mut bad = Vec::new();
    for (pin_key, dest) in FORWARD_STAGED_PINS {
        if !has_story_id_pattern(dest) {
            bad.push(format!("(\"{pin_key}\", \"{dest}\") — no S-NNN pattern"));
        }
    }
    assert!(
        bad.is_empty(),
        "FORWARD_STAGED_PINS entries without recognizable S-NNN story-id:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn no_orphan_pin_entries() {
    // Inverse check: every pin entry must map back to a registered lang OR be
    // explicitly forward-staged via FORWARD_STAGED_PINS. Catches stale entries
    // left after a lang is removed.
    let registered: BTreeSet<String> = registered_langs()
        .into_iter()
        .map(|m| mod_to_pin_key(&m).to_string())
        .collect();
    let staged: BTreeSet<&str> = FORWARD_STAGED_PINS.iter().map(|(k, _)| *k).collect();
    let pins = pin_keys();

    let orphans: Vec<&String> = pins
        .iter()
        .filter(|p| !registered.contains(*p) && !staged.contains(p.as_str()))
        .collect();
    assert!(
        orphans.is_empty(),
        "grammar-pins.toml has orphan entries (no matching mod in src/langs/mod.rs \
         and not in FORWARD_STAGED_PINS): {orphans:?}\n\n\
         If the entry is forward-staged for an imminent lang adapter, add it to \
         FORWARD_STAGED_PINS with a spec reference."
    );
}
