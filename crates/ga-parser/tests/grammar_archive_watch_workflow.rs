//! S-002 AS-005 — `.github/workflows/grammar-archive-watch.yml` structural validation.
//!
//! Asserts the workflow file exists at the expected path and contains the
//! key behavioral markers: daily cron schedule, pins.toml ingestion, gh api
//! call to repos/<upstream>, archived-field check, issue creation on
//! detection. Operational smoke ("known-archived test repo via manual
//! dispatch") is a manual-verification step, not covered here.

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

fn workflow_path() -> PathBuf {
    workspace_root().join(".github/workflows/grammar-archive-watch.yml")
}

fn load_workflow() -> String {
    fs::read_to_string(workflow_path()).unwrap_or_else(|e| {
        panic!(
            "grammar-archive-watch.yml must exist at {}: {e}\n\
             Per graphatlas-v1.2-grammar-pins.md S-002 AS-005.",
            workflow_path().display()
        )
    })
}

#[test]
fn workflow_file_exists_and_is_non_empty() {
    let raw = load_workflow();
    assert!(
        raw.len() > 100,
        "workflow file must be a real workflow, got {} bytes",
        raw.len()
    );
}

#[test]
fn workflow_has_daily_cron_schedule() {
    // AS-005.T1: scheduled daily 00:00 UTC. Cron syntax `0 0 * * *`.
    let raw = load_workflow();
    assert!(
        raw.contains("schedule:"),
        "workflow must have `schedule:` top-level key"
    );
    // Look for the cron line accepting either single or double quotes around the pattern.
    let has_cron = raw.lines().any(|l| {
        let t = l.trim();
        t.starts_with("- cron:")
            && (t.contains("'0 0 * * *'") || t.contains("\"0 0 * * *\"") || t.contains("0 0 * * *"))
    });
    assert!(
        has_cron,
        "workflow must schedule daily at 00:00 UTC via cron `0 0 * * *`"
    );
}

#[test]
fn workflow_supports_manual_dispatch() {
    // AS-005.T1: also triggered on manual dispatch — for ad-hoc reverification
    // (e.g., after a maintainer is alerted via other channel).
    let raw = load_workflow();
    assert!(
        raw.contains("workflow_dispatch"),
        "workflow must allow manual dispatch via `workflow_dispatch:` trigger"
    );
}

#[test]
fn workflow_reads_grammar_pins_toml() {
    // AS-005.T2: workflow parses grammar-pins.toml to enumerate entries.
    let raw = load_workflow();
    assert!(
        raw.contains("grammar-pins.toml"),
        "workflow must reference `grammar-pins.toml` to iterate entries"
    );
}

#[test]
fn workflow_calls_gh_api_repos_endpoint() {
    // AS-005.T2: iterates entries + calls `gh api repos/<upstream>`.
    let raw = load_workflow();
    let mentions_gh_api = raw.contains("gh api") || raw.contains("api.github.com/repos/");
    assert!(
        mentions_gh_api,
        "workflow must call `gh api repos/<upstream>` (or equivalent HTTP) to check repo state"
    );
}

#[test]
fn workflow_checks_archived_field() {
    // AS-005.T2: reads .archived boolean from API response.
    let raw = load_workflow();
    assert!(
        raw.contains(".archived") || raw.contains("'archived'") || raw.contains("\"archived\""),
        "workflow must inspect the `archived` field of repos/<upstream> response"
    );
}

#[test]
fn workflow_creates_issue_on_detection() {
    // AS-005.T3: auto-files an issue on archive detection.
    let raw = load_workflow();
    let creates_issue =
        raw.contains("gh issue create") || raw.contains("issues.create") || raw.contains("--issue");
    assert!(
        creates_issue,
        "workflow must create a GitHub issue on archive detection (gh issue create / API)"
    );
}

#[test]
fn workflow_does_not_modify_cargo_build_state() {
    // AS-005.T4: observe-only — workflow does NOT touch Cargo.toml / Cargo.lock /
    // grammar-pins.toml. Any modification would silently bypass the maintainer-
    // response window.
    let raw = load_workflow();
    let forbidden_writes = ["cargo update", "cargo add", "sed -i", "rm -", ">> Cargo"];
    for needle in forbidden_writes {
        assert!(
            !raw.contains(needle),
            "workflow must be observe-only — found forbidden modification: '{needle}'"
        );
    }
}
