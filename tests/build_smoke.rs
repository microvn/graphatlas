//! S-001 AS-001: confirm the binary compiles + answers --version + --help.
//! Extended in S-002 to include subcommand coverage.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_graphatlas")
}

#[test]
fn version_flag_exits_zero() {
    let out = Command::new(bin())
        .arg("--version")
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "exit: {:?}, stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("graphatlas"), "stdout: {stdout}");
    assert!(stdout.contains("0.1.0"), "stdout: {stdout}");
}

#[test]
fn help_mentions_eight_subcommands() {
    let out = Command::new(bin()).arg("--help").output().expect("spawn");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    for sub in [
        "mcp", "init", "doctor", "install", "list", "bench", "update", "cache",
    ] {
        assert!(
            s.contains(sub),
            "missing subcommand `{sub}` in --help output:\n{s}"
        );
    }
}

#[test]
fn update_prints_manual_instructions_exits_zero() {
    // AS-020: self-update deferred v1.1. Output must hit all 3 spec-literal
    // beats: (1) "deferred to v1.1", (2) a github releases URL, (3) a pointer
    // back to install.sh.
    let out = Command::new(bin()).arg("update").output().expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("v1.1"), "missing v1.1: {s}");
    assert!(s.contains("releases"), "missing releases link: {s}");
    assert!(
        s.to_lowercase().contains("install.sh"),
        "missing install.sh pointer: {s}"
    );
    assert!(
        s.to_lowercase().contains("deferred"),
        "missing 'deferred': {s}"
    );
}

#[test]
fn stub_subcommands_exit_zero_with_hint() {
    // `list` graduated in S-003. `doctor` and `install` graduated in S-002.
    // `mcp` graduated in S-006 (rmcp stdio transport, infra:S-003 v1.1-M0).
    // `init`, `cache` still stubs pending later milestones.
    for sub in ["init", "cache"] {
        let out = Command::new(bin()).arg(sub).output().expect("spawn");
        assert!(out.status.success(), "{sub} failed: {:?}", out);
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("S-001 stub"), "{sub} stdout: {s}");
    }
}

#[test]
fn list_subcommand_empty_cache_root() {
    // AS-028: with GRAPHATLAS_CACHE_DIR pointing at a nonexistent dir, `list`
    // prints the "(no caches under …)" message rather than erroring.
    let tmp = tempfile::tempdir().expect("tempdir");
    let nonexistent = tmp.path().join(".graphatlas");
    let out = Command::new(bin())
        .arg("list")
        .env("GRAPHATLAS_CACHE_DIR", &nonexistent)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("no caches under"), "stdout: {s}");
}

#[test]
fn list_subcommand_shows_populated_caches() {
    // AS-028 full row: populate 2 caches via the library, then run the CLI and
    // scrape the output for the sample shape shown in the spec.
    //
    // v1.5 PR2 AS-001: `commit()` strictly populates `indexed_root_hash` via
    // `compute_root_hash(repo_root)` — repo paths MUST exist on disk.
    use ga_index::Store;

    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_root = tmp.path().join(".graphatlas");
    let repo_a = tmp.path().join("repos").join("billing-api");
    let repo_b = tmp.path().join("repos").join("notes-app");
    for p in [&repo_a, &repo_b] {
        std::fs::create_dir_all(p).unwrap();
        std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
        Store::open_with_root(&cache_root, p).unwrap().commit().unwrap();
    }
    let out = Command::new(bin())
        .arg("list")
        .env("GRAPHATLAS_CACHE_DIR", &cache_root)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("NAME"), "header missing: {s}");
    assert!(s.contains("REPO PATH"), "header missing: {s}");
    assert!(s.contains("SIZE"), "header missing: {s}");
    assert!(s.contains("LAST INDEXED"), "header missing: {s}");
    // Row content: the canonical path strings are present (whatever
    // tempdir resolved them to).
    let repo_a_str = repo_a.display().to_string();
    let repo_b_str = repo_b.display().to_string();
    assert!(s.contains(&repo_a_str), "row missing for {repo_a_str}: {s}");
    assert!(s.contains(&repo_b_str), "row missing for {repo_b_str}: {s}");
}
