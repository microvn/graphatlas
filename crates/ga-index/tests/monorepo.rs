//! S-003 AS-021 + AS-022 — monorepo detection.
//!
//! Precedence per R39:
//!   1. Cargo.toml [workspace]
//!   2. pnpm-workspace.yaml
//!   3. nx.json
//!   4. lerna.json
//!   5. multi go.mod / package.json heuristic (≥3 sibling manifests, none in exclude list)
//!
//! Regression guard (AS-021 Data): a Next.js-style repo with hundreds of
//! node_modules/*/package.json must still be classified as FLAT.

use ga_index::monorepo::{detect, LayoutKind};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn touch(p: &Path) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, "").unwrap();
}

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

#[test]
fn empty_repo_is_flat() {
    let tmp = TempDir::new().unwrap();
    let kind = detect(tmp.path()).unwrap();
    assert!(matches!(kind, LayoutKind::Flat), "kind: {kind:?}");
}

#[test]
fn single_package_repo_is_flat() {
    // AS-022: one package.json at root → flat.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("package.json"), r#"{"name":"app"}"#);
    let kind = detect(tmp.path()).unwrap();
    assert!(matches!(kind, LayoutKind::Flat), "kind: {kind:?}");
}

#[test]
fn cargo_workspace_detected() {
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["crates/a", "crates/b"]"#,
    );
    write(
        &tmp.path().join("crates/a/Cargo.toml"),
        r#"[package]
name = "a""#,
    );
    write(
        &tmp.path().join("crates/b/Cargo.toml"),
        r#"[package]
name = "b""#,
    );

    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "cargo-workspace"),
        other => panic!("expected Monorepo(cargo-workspace), got {other:?}"),
    }
}

#[test]
fn non_workspace_cargo_toml_is_flat() {
    // plain [package] Cargo.toml is NOT a workspace.
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("Cargo.toml"),
        r#"[package]
name = "single""#,
    );
    let kind = detect(tmp.path()).unwrap();
    assert!(matches!(kind, LayoutKind::Flat), "kind: {kind:?}");
}

#[test]
fn pnpm_workspace_detected() {
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("pnpm-workspace.yaml"),
        r#"packages:
  - packages/*"#,
    );
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "pnpm-workspace"),
        other => panic!("expected pnpm, got {other:?}"),
    }
}

#[test]
fn nx_detected_after_pnpm_absence() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("nx.json"), "{}");
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "nx"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn lerna_detected() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("lerna.json"), "{}");
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "lerna"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn heuristic_3_siblings_with_package_json_detected() {
    // Three sibling dirs, each with package.json, none in exclude list.
    let tmp = TempDir::new().unwrap();
    for name in ["apps/web", "apps/api", "apps/worker"] {
        write(
            &tmp.path().join(name).join("package.json"),
            r#"{"name":"x"}"#,
        );
    }
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "heuristic-multi-manifest"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn heuristic_rejects_when_only_2_siblings() {
    let tmp = TempDir::new().unwrap();
    for name in ["a", "b"] {
        write(
            &tmp.path().join(name).join("package.json"),
            r#"{"name":"x"}"#,
        );
    }
    let kind = detect(tmp.path()).unwrap();
    assert!(matches!(kind, LayoutKind::Flat), "kind: {kind:?}");
}

#[test]
fn node_modules_false_positive_guard() {
    // AS-021 regression guard: Next.js-style flat repo with many
    // node_modules/*/package.json must still be FLAT.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("package.json"), r#"{"name":"nextjs-app"}"#);
    for name in ["react", "next", "lodash", "webpack", "typescript", "eslint"] {
        write(
            &tmp.path()
                .join("node_modules")
                .join(name)
                .join("package.json"),
            r#"{"name":"x"}"#,
        );
    }
    let kind = detect(tmp.path()).unwrap();
    assert!(
        matches!(kind, LayoutKind::Flat),
        "node_modules must not trigger monorepo detection; got {kind:?}"
    );
}

#[test]
fn excluded_dirs_skipped_in_heuristic() {
    // 3 package.json but all under excluded paths → still flat.
    let tmp = TempDir::new().unwrap();
    for name in ["vendor/a", "testdata/b", "examples/c"] {
        write(
            &tmp.path().join(name).join("package.json"),
            r#"{"name":"x"}"#,
        );
    }
    let kind = detect(tmp.path()).unwrap();
    assert!(matches!(kind, LayoutKind::Flat), "kind: {kind:?}");
}

#[test]
fn cargo_workspace_beats_pnpm_precedence() {
    // Both markers present — Cargo wins (higher precedence).
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["a"]"#,
    );
    write(
        &tmp.path().join("pnpm-workspace.yaml"),
        r#"packages:
  - x/*"#,
    );
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "cargo-workspace"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn missing_repo_is_error() {
    let kind = detect(Path::new("/nonexistent/path/really-not-there")).unwrap_err();
    let _ = kind;
}

#[test]
fn go_mod_heuristic() {
    let tmp = TempDir::new().unwrap();
    for name in ["svc/auth", "svc/billing", "svc/inventory"] {
        write(&tmp.path().join(name).join("go.mod"), "module x\n");
    }
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "heuristic-multi-manifest"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn empty_file_does_not_panic() {
    // Edge case: pnpm-workspace.yaml present but empty → still detects as pnpm.
    let tmp = TempDir::new().unwrap();
    touch(&tmp.path().join("pnpm-workspace.yaml"));
    let kind = detect(tmp.path()).unwrap();
    match kind {
        LayoutKind::Monorepo { marker, .. } => assert_eq!(marker, "pnpm-workspace"),
        other => panic!("{other:?}"),
    }
}
