//! S-002 — Phase 1+2 MVP enforcement of anti-tautology policy.
//!
//! Per spec:
//! - AS-005: rule files have anti-tautology policy header (convention).
//! - AS-006: `scripts/check-anti-tautology.sh` greps `crates/ga-bench/src/gt_gen/h*.rs`
//!   for forbidden `ga_query::{dead_code|callers|rename_safety|architecture|risk|minimal_context}`
//!   imports — exit 1 with clear message if any match.
//!
//! Build-time lint hardening (build.rs) is deferred to Phase 3 per spec
//! §"Not in Scope" — these tests pin the script-based MVP only.

use std::process::Command;
use tempfile::tempdir;

fn script_path() -> std::path::PathBuf {
    // Workspace root = parent of crates/ga-bench
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("scripts/check-anti-tautology.sh")
}

#[test]
fn as_006_script_passes_on_clean_repo() {
    let script = script_path();
    assert!(
        script.is_file(),
        "scripts/check-anti-tautology.sh must exist; got {script:?}"
    );

    // Run against the real repo — current rules use only allowed
    // `ga_query::common::*` and `ga_query::import_resolve::*`, so this is
    // expected to pass.
    let out = Command::new("bash")
        .arg(&script)
        .output()
        .expect("script must execute");
    assert!(
        out.status.success(),
        "script must pass on clean repo; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn as_006_script_fails_when_forbidden_import_present() {
    let script = script_path();
    let dir = tempdir().unwrap();
    // Mirror the gt_gen layout the script expects.
    let gt_gen = dir.path().join("crates/ga-bench/src/gt_gen");
    std::fs::create_dir_all(&gt_gen).unwrap();
    std::fs::write(
        gt_gen.join("h99_bogus.rs"),
        b"//! Bogus rule.\nuse ga_query::dead_code::DeadCodeEntry;\n",
    )
    .unwrap();

    let out = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", dir.path())
        .output()
        .expect("script must execute");
    assert!(
        !out.status.success(),
        "script must FAIL when a rule imports `ga_query::dead_code`; stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("anti-tautology") || combined.contains("Anti-tautology"),
        "failure output must reference the anti-tautology policy; got: {combined}"
    );
    assert!(
        combined.contains("h99_bogus.rs"),
        "failure output must name the offending file; got: {combined}"
    );
}

#[test]
fn as_006_script_fails_for_each_forbidden_module() {
    let script = script_path();
    for forbidden in [
        "dead_code",
        "callers",
        "rename_safety",
        "architecture",
        "risk",
        "minimal_context",
    ] {
        let dir = tempdir().unwrap();
        let gt_gen = dir.path().join("crates/ga-bench/src/gt_gen");
        std::fs::create_dir_all(&gt_gen).unwrap();
        let body = format!("use ga_query::{forbidden}::Something;\n");
        std::fs::write(gt_gen.join(format!("h99_{forbidden}.rs")), body).unwrap();

        let out = Command::new("bash")
            .arg(&script)
            .env("REPO_ROOT", dir.path())
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "script must FAIL for forbidden `ga_query::{forbidden}`; output: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn as_006_script_allows_ga_query_common_helpers() {
    let script = script_path();
    let dir = tempdir().unwrap();
    let gt_gen = dir.path().join("crates/ga-bench/src/gt_gen");
    std::fs::create_dir_all(&gt_gen).unwrap();
    // Allowed substrate (per spec policy header text): ga_parser, ga_store,
    // ga_query::common helpers. Existing rules already do this.
    std::fs::write(
        gt_gen.join("h99_clean.rs"),
        b"//! Clean rule.\nuse ga_query::common::is_test_path;\n",
    )
    .unwrap();

    let out = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "script must PASS when only `ga_query::common::*` is imported; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
