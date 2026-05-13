//! S-004 AS-031 — indexer walk hardening.
//!
//! 1. `symlink_metadata` used (not `metadata`) so symlinks are detected, not
//!    transparently followed.
//! 2. Paths whose canonical form escapes `repo_root` are skipped with warning.
//! 3. Default-exclude secret-shaped files: `.env*`, `.ssh/**`, `id_rsa*`,
//!    `*.pem`, `*.key`. Plus directory excludes from monorepo layer.

use ga_parser::walk::{walk_repo, WalkEntry, WalkReport};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn paths(entries: &[WalkEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| e.rel_path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn empty_repo_produces_empty_walk() {
    let tmp = TempDir::new().unwrap();
    let report = walk_repo(tmp.path()).unwrap();
    assert!(report.entries.is_empty());
    assert!(report.skipped_symlinks.is_empty());
    assert!(report.skipped_secrets.is_empty());
}

#[test]
fn walks_source_files_across_langs() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "print(1)");
    write(&tmp.path().join("lib.ts"), "export const x = 1;");
    write(&tmp.path().join("mod.go"), "package main");
    write(&tmp.path().join("src/main.rs"), "fn main() {}");
    write(&tmp.path().join("README.md"), "# not source"); // filtered out

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert!(names.iter().any(|p| p.ends_with("app.py")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("lib.ts")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("mod.go")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("main.rs")), "{names:?}");
    assert!(!names.iter().any(|p| p.ends_with("README.md")), "{names:?}");
}

#[test]
fn skips_default_excluded_dirs() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join("node_modules/react/index.js"), "");
    write(&tmp.path().join("target/debug/junk.rs"), "");
    write(&tmp.path().join("vendor/lib/lib.go"), "");
    write(&tmp.path().join(".git/hooks/post-merge"), "");

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert_eq!(names.len(), 1, "only app.py should survive: {names:?}");
    assert!(names[0].ends_with("app.py"));
}

#[test]
fn skips_secret_shaped_files() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join(".env"), "SECRET=1");
    write(&tmp.path().join(".env.local"), "SECRET=2");
    write(&tmp.path().join("deploy.pem"), "-----BEGIN-----");
    write(&tmp.path().join("deploy.key"), "-----BEGIN-----");
    write(&tmp.path().join("id_rsa"), "-----BEGIN-----");
    write(&tmp.path().join("id_rsa.pub"), "-----BEGIN-----");

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert_eq!(names.len(), 1, "only app.py should survive: {names:?}");

    let skipped: Vec<String> = report
        .skipped_secrets
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(skipped.contains(&".env".to_string()), "{skipped:?}");
    assert!(skipped.contains(&".env.local".to_string()), "{skipped:?}");
    assert!(skipped.contains(&"deploy.pem".to_string()), "{skipped:?}");
    assert!(skipped.contains(&"deploy.key".to_string()), "{skipped:?}");
    assert!(skipped.contains(&"id_rsa".to_string()), "{skipped:?}");
}

#[cfg(unix)]
#[test]
fn skips_symlink_escaping_repo_root() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("app.py"), "# legit");

    // External target outside the repo.
    let external_dir = tmp.path().join("outside");
    fs::create_dir_all(&external_dir).unwrap();
    write(&external_dir.join("target.py"), "# outside repo");

    // docs/ref -> ../outside (escapes repo_root).
    fs::create_dir_all(repo.join("docs")).unwrap();
    symlink(&external_dir, repo.join("docs/ref")).unwrap();

    let report = walk_repo(&repo).unwrap();
    let names = paths(&report.entries);
    assert!(names.iter().any(|p| p.ends_with("app.py")), "{names:?}");
    assert!(
        !names.iter().any(|p| p.contains("outside")),
        "symlink target leaked: {names:?}"
    );
    // Report should surface the skipped symlink.
    assert!(
        !report.skipped_symlinks.is_empty(),
        "expected skipped_symlinks to include docs/ref"
    );
}

#[cfg(unix)]
#[test]
fn skips_symlink_to_absolute_sensitive_path() {
    // Adversarial: symlink pointing at /etc/passwd.
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    symlink("/etc/passwd", tmp.path().join("leak.py")).unwrap();
    write(&tmp.path().join("real.py"), "");

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    // leak.py is a symlink to outside the repo → skipped. real.py survives.
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with("real.py"));
}

#[cfg(unix)]
#[test]
fn symlink_into_excluded_dir_is_rejected() {
    // Regression: crates/ga-parser/src/walk.rs EXCLUDED_DIRS check compared
    // file_name against names list, so a symlink `shortcut -> target/debug/`
    // snuck past — walker recursed into target/debug and indexed files
    // inside an excluded subtree.
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    // Populate an excluded subtree with a source file.
    write(&tmp.path().join("target/debug/sneaky.rs"), "fn x() {}");
    // Innocent-looking symlink name → excluded canonical target.
    symlink(tmp.path().join("target/debug"), tmp.path().join("shortcut")).unwrap();

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert!(
        !names.iter().any(|p| p.contains("sneaky.rs")),
        "symlink into excluded dir should not leak files: {names:?}"
    );
    assert!(names.iter().any(|p| p.ends_with("app.py")), "{names:?}");
}

#[test]
fn walk_returns_relative_paths_to_repo_root() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("sub/deep/app.py"), "");
    let report = walk_repo(tmp.path()).unwrap();
    assert_eq!(report.entries.len(), 1);
    assert_eq!(
        report.entries[0].rel_path.to_string_lossy(),
        "sub/deep/app.py"
    );
}

#[test]
fn walk_missing_repo_is_error() {
    let err = walk_repo(Path::new("/nonexistent/path/really")).err();
    assert!(err.is_some());
}

#[test]
fn walk_report_default_constructs() {
    let _r = WalkReport::default();
}

// 2026-05-03 walker exclusion overhaul — see crates/ga-parser/src/walk.rs.
// Three properties to guard against regression:
//   1. The expanded EXCLUDED_DIRS skips ecosystem package-manager / build
//      caches that the previous list missed. Repos with `.next/`,
//      `__pycache__/`, `.gradle/`, `.idea/` no longer pay to index them.
//   2. `examples/`, `fixtures/`, `testdata/` are NOT excluded by default.
//      They contain real source the user wants in the call graph; users
//      who do want them skipped say so via `.gitignore`.
//   3. `.gitignore` is honoured for project-specific exclusions, which is
//      how `.graphatlas-bench-cache/` and similar finally stop slowing
//      first-time indexing in graphatlas's own repo.

#[test]
fn skips_expanded_universal_junk_dirs() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    // One file inside each newly-excluded dir.
    for junk in [
        ".next/foo.js",
        "__pycache__/foo.cpython-311.pyc",
        ".pytest_cache/v/cache/lastfailed",
        ".gradle/foo.gradle",
        ".idea/workspace.xml",
        ".turbo/foo.json",
        ".cache/foo",
        "coverage/lcov.info",
        ".dart_tool/foo.dart",
        ".m2/repository/foo.jar",
        "_build/foo.beam",
        "DerivedData/foo.swift",
    ] {
        write(&tmp.path().join(junk), "");
    }

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    // Only app.py is a recognised source extension; the others wouldn't be
    // indexed anyway because of language detection. The point of the
    // test is that the WALK doesn't recurse into these dirs (cheap to
    // verify proxy: assert no non-.py file appears under these prefixes,
    // and in the case of `_build/foo.beam` etc. nothing surfaces at all).
    assert_eq!(names.len(), 1, "only app.py should survive: {names:?}");
    assert!(names[0].ends_with("app.py"));
}

#[test]
fn examples_fixtures_testdata_are_indexed_now() {
    // Pre-2026-05-03 these dirs were in EXCLUDED_DIRS, which skipped
    // legitimate source. Regression guard: keep them indexed.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("examples/hello.rs"), "fn main() {}");
    write(&tmp.path().join("fixtures/seed.py"), "x = 1");
    write(&tmp.path().join("testdata/case.go"), "package x");

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert!(names.iter().any(|p| p.ends_with("hello.rs")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("seed.py")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("case.go")), "{names:?}");
}

#[test]
fn respects_root_gitignore() {
    let tmp = TempDir::new().unwrap();
    // Project-specific cache dir — neither in EXCLUDED_DIRS nor a
    // recognised package-manager folder. Only `.gitignore` keeps it out.
    write(&tmp.path().join(".gitignore"), ".my-cache/\n*.gen.ts\n");
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join(".my-cache/dump.py"), "");
    write(&tmp.path().join("src/code.ts"), "");
    write(&tmp.path().join("src/code.gen.ts"), "");

    let report = walk_repo(tmp.path()).unwrap();
    let names = paths(&report.entries);
    assert!(names.iter().any(|p| p.ends_with("app.py")), "{names:?}");
    assert!(names.iter().any(|p| p.ends_with("code.ts")), "{names:?}");
    assert!(
        !names.iter().any(|p| p.contains(".my-cache")),
        "gitignored dir leaked: {names:?}"
    );
    assert!(
        !names.iter().any(|p| p.ends_with("code.gen.ts")),
        "gitignored file leaked: {names:?}"
    );
}

#[test]
fn missing_gitignore_does_not_break_walk() {
    // GitignoreBuilder::add silently no-ops on missing files, but guard
    // the contract — repos without `.gitignore` must still index normally.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("a.py"), "");
    write(&tmp.path().join("b.rs"), "");
    let report = walk_repo(tmp.path()).unwrap();
    assert_eq!(report.entries.len(), 2);
}
