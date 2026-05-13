//! Co-change miner — git-log-based signal.
//!
//! Ported from `src/adapters/co-change.ts`. Thesis: hub files (utils, base
//! classes) appear in callers/importers for most tasks, but commit history
//! rarely modifies them together with the target. Co-change rate
//! distinguishes true context from hub noise — orthogonal with signal
//! counting.
//!
//! Walks recent commits touching `target_file`, collects sibling files per
//! commit, skips monster commits (>max_commit_size files — rename / refactor
//! noise), returns `file → co-change count` map.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

/// Default N commits scanned — matches TS default.
pub const DEFAULT_N_COMMITS: u32 = 200;

/// Default commit-size threshold above which the commit is ignored.
pub const DEFAULT_MAX_COMMIT_SIZE: usize = 50;

/// Commit-hash + sibling-file map for `target_file`. Empty on any git failure.
pub fn get_co_change_files(
    repo_path: &Path,
    target_file: &str,
    n_commits: u32,
    max_commit_size: usize,
) -> HashMap<String, u32> {
    let mut result: HashMap<String, u32> = HashMap::new();
    if target_file.is_empty() {
        return result;
    }

    // Step 1 — commits touching target_file.
    let hashes_output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args([
            "log",
            "--no-merges",
            &format!("-n{n_commits}"),
            "--format=%H",
            "--",
            target_file,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(hashes_out) = hashes_output else {
        return result;
    };
    if !hashes_out.status.success() {
        return result;
    }
    let Ok(hashes_text) = std::str::from_utf8(&hashes_out.stdout) else {
        return result;
    };
    let hashes: Vec<&str> = hashes_text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if hashes.is_empty() {
        return result;
    }

    // Step 2 — per-commit file listings via `git log --no-walk`. tformat:
    // prefix required — raw `===FOO===` fails with "invalid --pretty format".
    let mut args: Vec<&str> = vec![
        "log",
        "--no-walk",
        "--name-only",
        "--format=tformat:===COMMIT===",
    ];
    args.extend(hashes.iter());
    let show_output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(show_out) = show_output else {
        return result;
    };
    if !show_out.status.success() {
        return result;
    }
    let Ok(text) = std::str::from_utf8(&show_out.stdout) else {
        return result;
    };

    for chunk in text.split("===COMMIT===") {
        let files: Vec<&str> = chunk
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if files.is_empty() || files.len() > max_commit_size {
            continue;
        }
        for f in files {
            if f == target_file {
                continue;
            }
            *result.entry(f.to_string()).or_insert(0) += 1;
        }
    }
    result
}

/// Convert a co-change map into a descending-by-count ranked list, capped
/// at `limit`. Tie-break alphabetically for determinism (TS version was
/// non-deterministic across runs due to Map insertion order).
pub fn co_change_ranked_list(map: &HashMap<String, u32>, limit: usize) -> Vec<String> {
    let mut entries: Vec<(String, u32)> = map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.into_iter().take(limit).map(|(p, _)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranked_list_orders_by_count_desc() {
        let mut m = HashMap::new();
        m.insert("a.rs".into(), 3);
        m.insert("b.rs".into(), 10);
        m.insert("c.rs".into(), 5);
        let r = co_change_ranked_list(&m, 10);
        assert_eq!(r, vec!["b.rs", "c.rs", "a.rs"]);
    }

    #[test]
    fn ranked_list_breaks_ties_alphabetically() {
        let mut m = HashMap::new();
        m.insert("z.rs".into(), 5);
        m.insert("a.rs".into(), 5);
        m.insert("m.rs".into(), 5);
        let r = co_change_ranked_list(&m, 10);
        assert_eq!(r, vec!["a.rs", "m.rs", "z.rs"]);
    }

    #[test]
    fn ranked_list_respects_limit() {
        let mut m = HashMap::new();
        for i in 0..10 {
            m.insert(format!("f{i}.rs"), 10 - i as u32);
        }
        let r = co_change_ranked_list(&m, 3);
        assert_eq!(r.len(), 3);
        assert_eq!(r, vec!["f0.rs", "f1.rs", "f2.rs"]);
    }

    #[test]
    fn ranked_list_empty_map_yields_empty() {
        let r = co_change_ranked_list(&HashMap::new(), 10);
        assert!(r.is_empty());
    }

    #[test]
    fn get_co_change_files_empty_target_returns_empty_map() {
        let tmp = tempfile::TempDir::new().unwrap();
        let r = get_co_change_files(tmp.path(), "", DEFAULT_N_COMMITS, DEFAULT_MAX_COMMIT_SIZE);
        assert!(r.is_empty());
    }

    #[test]
    fn get_co_change_files_non_git_dir_returns_empty_map() {
        let tmp = tempfile::TempDir::new().unwrap();
        let r = get_co_change_files(
            tmp.path(),
            "foo.rs",
            DEFAULT_N_COMMITS,
            DEFAULT_MAX_COMMIT_SIZE,
        );
        assert!(r.is_empty());
    }
}
