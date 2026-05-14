//! S-002 AS-006 — recovery protocol doc structural validation.
//!
//! Asserts `docs/guide/grammar-archival-recovery.md` exists, contains the
//! mandatory 3-step protocol, has a worked example, and is reachable from
//! `grammar-pins.toml` header comment (cross-link).

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

fn recovery_doc() -> PathBuf {
    workspace_root().join("docs/guide/grammar-archival-recovery.md")
}

fn pins_toml() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("grammar-pins.toml")
}

#[test]
fn recovery_doc_exists() {
    assert!(
        recovery_doc().exists(),
        "recovery protocol doc must exist at {}",
        recovery_doc().display()
    );
}

#[test]
fn recovery_doc_has_three_step_protocol() {
    // AS-006.T1: doc documents 3-step recovery protocol.
    let raw = fs::read_to_string(recovery_doc()).expect("read recovery doc");
    // Look for markers of all three numbered steps. Be tolerant of formatting
    // (e.g. `## Step 1` / `### 1.` / `**1.**`).
    for n in 1..=3 {
        let needles = [
            format!("Step {n}"),
            format!("step {n}"),
            format!("{n}. "),
            format!("**{n}.**"),
        ];
        let found = needles.iter().any(|needle| raw.contains(needle));
        assert!(found, "recovery doc must document step {n}");
    }
}

#[test]
fn recovery_doc_step1_mentions_canonical_fork_search() {
    // AS-006.T1 step 1: identify community canonical fork.
    let raw = fs::read_to_string(recovery_doc()).expect("read recovery doc");
    let lower = raw.to_ascii_lowercase();
    assert!(
        lower.contains("fork"),
        "step 1 must reference 'fork' (community canonical fork search)"
    );
    assert!(
        lower.contains("gh search") || lower.contains("stars") || lower.contains("github search"),
        "step 1 must show how to discover the canonical fork (gh search / stars / GitHub search)"
    );
}

#[test]
fn recovery_doc_step2_mentions_pins_toml_update() {
    // AS-006.T1 step 2: update grammar-pins.toml fields.
    let raw = fs::read_to_string(recovery_doc()).expect("read recovery doc");
    assert!(
        raw.contains("grammar-pins.toml"),
        "step 2 must explicitly reference grammar-pins.toml"
    );
    let lower = raw.to_ascii_lowercase();
    assert!(
        lower.contains("upstream") && lower.contains("commit"),
        "step 2 must mention which fields to update (upstream + commit)"
    );
    assert!(
        lower.contains("upstream_archive_replacement"),
        "step 2 must add `upstream_archive_replacement` field for audit trail"
    );
}

#[test]
fn recovery_doc_step3_mentions_regression_suite() {
    // AS-006.T1 step 3: regression suite passes.
    let raw = fs::read_to_string(recovery_doc()).expect("read recovery doc");
    let lower = raw.to_ascii_lowercase();
    assert!(
        lower.contains("cargo test") || lower.contains("regression"),
        "step 3 must reference the regression suite (cargo test / regression)"
    );
}

#[test]
fn recovery_doc_has_worked_example() {
    // AS-006.T2: doc has a worked example.
    let raw = fs::read_to_string(recovery_doc()).expect("read recovery doc");
    let lower = raw.to_ascii_lowercase();
    let has_example_marker = lower.contains("example") || lower.contains("worked");
    assert!(
        has_example_marker,
        "recovery doc must contain a worked example (look for 'example' or 'worked' section header)"
    );
}

#[test]
fn pins_toml_links_to_recovery_doc() {
    // AS-006.T3: pins.toml header comment links back to recovery doc.
    let raw = fs::read_to_string(pins_toml()).expect("read grammar-pins.toml");
    assert!(
        raw.contains("grammar-archival-recovery.md"),
        "grammar-pins.toml header must link to docs/guide/grammar-archival-recovery.md"
    );
}
