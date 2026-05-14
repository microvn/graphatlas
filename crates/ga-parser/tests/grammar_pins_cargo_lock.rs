//! S-001 AS-003 — every `[pins.<lang>].crate_version` in `grammar-pins.toml`
//! must match the resolved version in workspace `Cargo.lock`.
//!
//! Prevents pins from drifting silently when `cargo update` bumps a crate.
//! If they ever diverge, this test fails before bench/leaderboard numbers
//! ever shift on the affected lang.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // crates/ga-parser/tests → up 3 levels.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn ga_parser_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[derive(Debug, serde::Deserialize)]
struct PinFile {
    pins: BTreeMap<String, PinEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct PinEntry {
    #[serde(rename = "crate")]
    krate: String,
    crate_version: String,
    upstream: String,
    commit: String,
}

#[derive(Debug, serde::Deserialize)]
struct CargoLock {
    package: Vec<LockedPackage>,
}

#[derive(Debug, serde::Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
}

fn load_pins() -> PinFile {
    let raw = fs::read_to_string(ga_parser_root().join("grammar-pins.toml"))
        .expect("grammar-pins.toml must exist");
    toml::from_str(&raw).expect("grammar-pins.toml must parse")
}

fn load_lock() -> CargoLock {
    let raw = fs::read_to_string(workspace_root().join("Cargo.lock"))
        .expect("Cargo.lock must exist at workspace root");
    toml::from_str(&raw).expect("Cargo.lock must parse")
}

#[test]
fn pin_crate_version_matches_cargo_lock() {
    let pins = load_pins();
    let lock = load_lock();

    // Build crate-name → version map from lockfile.
    let mut locked: BTreeMap<String, String> = BTreeMap::new();
    for pkg in &lock.package {
        locked.insert(pkg.name.clone(), pkg.version.clone());
    }

    let mut mismatches = Vec::new();
    let mut missing_from_lock = Vec::new();
    for (lang, entry) in &pins.pins {
        match locked.get(&entry.krate) {
            None => missing_from_lock.push(format!(
                "[pins.{lang}].crate = '{}' (declared crate_version={}) not present in Cargo.lock — \
                 either the crate is not yet declared in Cargo.toml or the pin is stale",
                entry.krate, entry.crate_version
            )),
            Some(actual) if actual != &entry.crate_version => mismatches.push(format!(
                "[pins.{lang}]: crate_version={} but Cargo.lock has {}={}",
                entry.crate_version, entry.krate, actual
            )),
            Some(_) => {}
        }
    }

    assert!(
        mismatches.is_empty(),
        "grammar-pins.toml crate_version drift vs Cargo.lock:\n  {}",
        mismatches.join("\n  ")
    );
    assert!(
        missing_from_lock.is_empty(),
        "grammar-pins.toml references crates not in Cargo.lock:\n  {}",
        missing_from_lock.join("\n  ")
    );
}
