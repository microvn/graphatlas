//! C1 — independent Ruby gem load-path reader for the `architecture` GT.
//!
//! ## Anti-tautology policy (§C1)
//! Does NOT import `ga_query::*` (not even the engine's `ts_workspace`
//! resolver). Derives gem `lib` load-path roots from raw filesystem scan
//! (a dir holding a `.gemspec` → its `lib`), mirroring Ruby's `$LOAD_PATH`
//! semantics independently of the tool.
//!
//! Only the LOAD-PATH form (`require "x/y"`) is resolved here — that is where
//! inter-gem architecture edges live. `require_relative "./x"` is intra-gem
//! (relative to the requiring file) → same module → a self-edge the GT drops,
//! so it is intentionally left to the caller as `None`.

use std::collections::HashSet;
use std::path::Path;

/// Repo-relative `lib` dirs of every gem (a dir containing a `.gemspec`).
pub fn lib_roots(repo_root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(repo_root, repo_root, &mut out);
    out.sort();
    out.dedup();
    out
}

/// Resolve a bare `require "x/y"` to a repo-relative `.rb` file: the first gem
/// `lib` root holding `<root>/x/y.rb`. Existence-checked against `rb_files` so
/// the correct providing gem is selected. Relative (`./`, `../`) → `None`
/// (intra-gem, dropped by the caller).
pub fn resolve(lib_roots: &[String], raw: &str, rb_files: &HashSet<String>) -> Option<String> {
    if raw.starts_with("./") || raw.starts_with("../") {
        return None;
    }
    for root in lib_roots {
        let candidate = if root.is_empty() {
            format!("{raw}.rb")
        } else {
            format!("{root}/{raw}.rb")
        };
        if rb_files.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn walk(repo_root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut has_gemspec = false;
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                base,
                ".git" | "node_modules" | "target" | "dist" | "build" | "__pycache__"
            ) {
                continue;
            }
            subdirs.push(path);
        } else if path.extension().and_then(|x| x.to_str()) == Some("gemspec") {
            has_gemspec = true;
        }
    }
    if has_gemspec && dir.join("lib").is_dir() {
        let rel = dir
            .strip_prefix(repo_root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(if rel.is_empty() {
            "lib".to_string()
        } else {
            format!("{rel}/lib")
        });
    }
    for sub in subdirs {
        walk(repo_root, &sub, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn loadpath_resolves_to_providing_gem() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("activesupport/activesupport.gemspec"), "");
        write(&root.join("activesupport/lib/active_support.rb"), "");
        write(&root.join("activerecord/activerecord.gemspec"), "");
        write(&root.join("activerecord/lib/active_record.rb"), "");

        let roots = lib_roots(root);
        let files: HashSet<String> = [
            "activesupport/lib/active_support.rb".to_string(),
            "activerecord/lib/active_record.rb".to_string(),
        ]
        .into();
        // `require "active_support"` from activerecord → activesupport's lib.
        assert_eq!(
            resolve(&roots, "active_support", &files).as_deref(),
            Some("activesupport/lib/active_support.rb")
        );
    }

    #[test]
    fn relative_require_is_none() {
        let files: HashSet<String> = HashSet::new();
        assert_eq!(resolve(&["lib".to_string()], "./foo", &files), None);
    }

    #[test]
    fn external_gem_is_none() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("g/g.gemspec"), "");
        write(&root.join("g/lib/g.rb"), "");
        let roots = lib_roots(root);
        let files: HashSet<String> = ["g/lib/g.rb".to_string()].into();
        // `require "nokogiri"` — external, no in-repo lib provides it.
        assert_eq!(resolve(&roots, "nokogiri", &files), None);
    }
}
