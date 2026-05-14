//! v1.2-php S-001 AS-017 — composer.json PSR-4 path traversal hardening.
//!
//! Foundation-C16 Phase B import resolver currently does NOT read composer.json
//! (verified at /mf-build 2026-05-14 audit). AS-017 ships the security
//! primitive PREEMPTIVELY so that when a future PSR-4 reader story lands, it
//! inherits the canonicalization+escape-rejection logic for free.
//!
//! Test surface:
//! - `canonicalize_psr4_root(repo_root, raw)` — Ok(canonical) for paths
//!   inside repo_root; Err for escape.
//! - `read_composer_psr4(path, repo_root)` — parses composer.json `autoload.psr-4`,
//!   filters out hostile entries, surfaces `psr4_escape` warnings.
//!
//! Per OWASP A01:2021 (Path Traversal).

use std::fs;
use std::path::PathBuf;

use ga_query::psr4_resolve::{
    canonicalize_psr4_root, read_composer_psr4, Psr4Entry, Psr4ResolveError,
};
use tempfile::TempDir;

/// Returns `(tmp_dir_guard, repo_root, sibling_outside_repo)`. The composer.json
/// inside repo_root references both:
///   - `src/`   → inside repo (safe)
///   - `../sibling/` → real dir OUTSIDE repo_root inside tmp (hostile but real,
///                     so canonicalize succeeds → EscapesRepoRoot triggers)
fn setup_repo() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("tmp dir");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(tmp.path().join("sibling")).unwrap();
    fs::write(
        repo.join("composer.json"),
        r#"{
            "name": "test/repo",
            "autoload": {
                "psr-4": {
                    "App\\": "src/",
                    "Evil\\": "../sibling/"
                }
            }
        }"#,
    )
    .unwrap();
    (tmp, repo)
}

#[test]
fn canonicalize_accepts_inside_repo_root() {
    let (_tmp, repo) = setup_repo();
    let resolved = canonicalize_psr4_root(&repo, "src/").expect("src/ canonicalizes");
    assert!(
        resolved.starts_with(&repo.canonicalize().unwrap()),
        "resolved {} must start with canonicalized repo root {}",
        resolved.display(),
        repo.display()
    );
}

#[test]
fn canonicalize_rejects_dot_dot_escape() {
    let (_tmp, repo) = setup_repo();
    // ../sibling/ is a real dir OUTSIDE repo_root — canonicalize succeeds,
    // escape check MUST fire.
    let result = canonicalize_psr4_root(&repo, "../sibling/");
    assert!(
        matches!(result, Err(Psr4ResolveError::EscapesRepoRoot { .. })),
        "../sibling/ (real dir outside repo) MUST be rejected as escape, got {result:?}"
    );
}

#[test]
fn canonicalize_rejects_absolute_path_outside_root() {
    let (_tmp, repo) = setup_repo();
    let result = canonicalize_psr4_root(&repo, "/etc/");
    assert!(
        matches!(
            result,
            Err(Psr4ResolveError::EscapesRepoRoot { .. }) | Err(Psr4ResolveError::NotFound { .. })
        ),
        "/etc/ MUST be rejected (escape or not-found in tmp), got {result:?}"
    );
}

#[test]
fn canonicalize_rejects_nonexistent_path() {
    let (_tmp, repo) = setup_repo();
    let result = canonicalize_psr4_root(&repo, "nonexistent/");
    assert!(
        matches!(result, Err(Psr4ResolveError::NotFound { .. })),
        "nonexistent path MUST be rejected, got {result:?}"
    );
}

#[test]
fn read_composer_returns_only_safe_entries_with_warnings() {
    let (_tmp, repo) = setup_repo();
    let composer_path = repo.join("composer.json");

    let result = read_composer_psr4(&composer_path, &repo).expect("read OK");
    let safe: Vec<&Psr4Entry> = result.entries.iter().collect();

    // App\ → src/ is safe.
    let app_entry = safe.iter().find(|e| e.namespace == "App\\");
    assert!(
        app_entry.is_some(),
        "App\\ → src/ should pass through: {safe:?}"
    );

    // Evil\ → ../../../../etc/ must be filtered out.
    let evil_entry = safe.iter().find(|e| e.namespace == "Evil\\");
    assert!(
        evil_entry.is_none(),
        "Evil\\ → ../../../etc/ must be filtered: {safe:?}"
    );

    // Filtered entry must surface as warning so security consumers can audit.
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("psr4_escape") && w.contains("Evil")),
        "warnings must include psr4_escape for Evil\\: {:?}",
        result.warnings
    );
}

#[test]
fn read_composer_handles_missing_autoload_gracefully() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().to_path_buf();
    fs::write(
        repo.join("composer.json"),
        r#"{"name": "minimal/composer"}"#,
    )
    .unwrap();
    let result = read_composer_psr4(&repo.join("composer.json"), &repo).expect("read OK");
    assert!(
        result.entries.is_empty(),
        "composer.json without autoload → empty entries"
    );
    assert!(
        result.warnings.is_empty(),
        "composer.json without autoload → no warnings"
    );
}

#[test]
fn read_composer_rejects_malformed_json() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().to_path_buf();
    fs::write(repo.join("composer.json"), "{not valid json").unwrap();
    let result = read_composer_psr4(&repo.join("composer.json"), &repo);
    assert!(result.is_err(), "malformed JSON must surface as Err");
}

#[test]
fn fixture_composer_json_includes_hostile_entry() {
    // Sanity: the parser-test fixture composer.json contains the hostile entry
    // that AS-017 protects against. If the fixture is edited to remove the
    // hostile entry, the canary loses its data.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("crates/ga-parser/tests/fixtures/php-tiny/symfony-mini/composer.json");
    let content = fs::read_to_string(&fixture).expect("fixture composer.json must exist");
    assert!(
        content.contains("\"Evil\\\\\":")
            && (content.contains("../../") || content.contains("/etc/")),
        "fixture composer.json must contain hostile psr-4 entry for AS-017 to be meaningful"
    );
}
