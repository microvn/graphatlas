//! Shared helpers for integration tests.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Returns true if `bin` is callable as a subprocess (i.e. exists on PATH
/// and is executable). Matches what `std::process::Command::spawn` would
/// resolve, so it agrees with what retrievers see at runtime.
pub fn tool_present(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stderr.is_empty() || !o.stdout.is_empty())
        .unwrap_or(false)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// Small Python fixture + ground-truth JSON covering the `callers` UC.
/// Used across external-retriever smoke tests so they all bench against
/// the same corpus.
pub fn setup_mini_fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(
        &repo.join("auth.py"),
        "def check_password(u, p):\n    return u.pw == p\n\ndef login_view(u, p):\n    return check_password(u, p)\n\ndef api_login(u, p):\n    return check_password(u, p)\n",
    );
    write(&repo.join("utils.py"), "def fmt(x):\n    return str(x)\n");

    let gt_path = tmp.path().join("gt.json");
    fs::write(
        &gt_path,
        r#"{
            "schema_version": 1,
            "uc": "callers",
            "fixture": "mini-smoke",
            "tasks": [
                {"task_id":"check_password","query":{"symbol":"check_password"},"expected":["login_view","api_login"]}
            ]
        }"#,
    )
    .unwrap();
    (tmp, repo, gt_path)
}
