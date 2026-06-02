//! Rust crate-dependency scope for name resolution.
//!
//! The repo-wide name fallback (tier 3) resolves a bare call/reference to a
//! same-named symbol anywhere in the repo. On a Cargo workspace that lets a
//! library crate's call resolve into an unrelated crate (e.g. a `benches` /
//! `examples` / `tests` crate that merely reuses the name) — a cross-crate
//! over-link the caller could never actually make, since it does not depend on
//! that crate. This reads `cargo metadata` (offline) to know each member's
//! directory + its dependency crates, so the fallback only resolves within the
//! caller crate or a crate it genuinely depends on.
//!
//! Empty (⇒ no filtering) for non-Cargo repos or when `cargo metadata` is
//! unavailable — callers then fall back to plain path-proximity, so Python /
//! Go / TS resolution is unchanged.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Pkg>,
}

#[derive(Deserialize)]
struct Pkg {
    name: String,
    manifest_path: String,
    dependencies: Vec<Dep>,
}

#[derive(Deserialize)]
struct Dep {
    name: String,
}

/// Crate-membership + dependency closure for a Cargo workspace, keyed by
/// repo-relative crate root directory.
#[derive(Default)]
pub struct CrateScope {
    /// Crate root dirs (repo-relative), longest-first for prefix matching.
    roots: Vec<String>,
    /// crate-root-dir → allowed target crate-root-dirs (itself + its deps).
    allowed: HashMap<String, HashSet<String>>,
}

impl CrateScope {
    /// Build from `cargo metadata`. Returns an empty scope (no filtering) when
    /// `repo_root` is not a Cargo workspace or cargo is unavailable / offline
    /// metadata fails.
    pub fn load(repo_root: &Path) -> Self {
        let out = std::process::Command::new("cargo")
            .args([
                "metadata",
                "--no-deps",
                "--offline",
                "--format-version",
                "1",
            ])
            .current_dir(repo_root)
            .output();
        let stdout = match out {
            Ok(o) if o.status.success() => o.stdout,
            _ => return Self::default(),
        };
        let meta: Metadata = match serde_json::from_slice(&stdout) {
            Ok(m) => m,
            Err(_) => return Self::default(),
        };
        // Canonicalize so the prefix matches cargo's canonical `manifest_path`
        // (on macOS `/var` vs `/private/var` would otherwise fail to strip).
        let root_abs = std::fs::canonicalize(repo_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| repo_root.to_string_lossy().replace('\\', "/"));
        let rel_dir = |manifest_path: &str| -> String {
            let abs_dir = Path::new(manifest_path)
                .parent()
                .map(|d| d.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            abs_dir
                .strip_prefix(&root_abs)
                .map(|s| s.trim_start_matches('/').to_string())
                .unwrap_or(abs_dir)
        };

        let mut name_dir: HashMap<String, String> = HashMap::new();
        for p in &meta.packages {
            name_dir.insert(p.name.clone(), rel_dir(&p.manifest_path));
        }
        let mut allowed: HashMap<String, HashSet<String>> = HashMap::new();
        for p in &meta.packages {
            let me = name_dir.get(&p.name).cloned().unwrap_or_default();
            let set = allowed.entry(me.clone()).or_default();
            set.insert(me);
            for d in &p.dependencies {
                if let Some(dep_dir) = name_dir.get(&d.name) {
                    set.insert(dep_dir.clone());
                }
            }
        }
        let mut roots: Vec<String> = name_dir.into_values().filter(|r| !r.is_empty()).collect();
        roots.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        roots.dedup();
        CrateScope { roots, allowed }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    fn crate_of<'a>(&'a self, file: &str) -> Option<&'a str> {
        self.roots
            .iter()
            .find(|r| file == r.as_str() || file.starts_with(&format!("{r}/")))
            .map(|s| s.as_str())
    }

    /// May a caller in `caller_file` resolve a name to a definition in
    /// `candidate_file`? True when there is no crate info (don't filter),
    /// either file is outside any crate, both are the same crate, or the
    /// caller's crate depends on the candidate's crate.
    pub fn allows(&self, caller_file: &str, candidate_file: &str) -> bool {
        if self.roots.is_empty() {
            return true;
        }
        let (Some(cc), Some(tc)) = (self.crate_of(caller_file), self.crate_of(candidate_file))
        else {
            return true;
        };
        if cc == tc {
            return true;
        }
        self.allowed.get(cc).map(|s| s.contains(tc)).unwrap_or(true)
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
    fn allows_dependency_blocks_non_dependency() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\", \"c\"]\nresolver = \"2\"\n",
        );
        write(&root.join("a/Cargo.toml"), "[package]\nname=\"a\"\nversion=\"0.1.0\"\nedition=\"2021\"\n\n[dependencies]\nb={path=\"../b\"}\n");
        write(&root.join("a/src/lib.rs"), "");
        write(
            &root.join("b/Cargo.toml"),
            "[package]\nname=\"b\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        );
        write(&root.join("b/src/lib.rs"), "");
        write(
            &root.join("c/Cargo.toml"),
            "[package]\nname=\"c\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        );
        write(&root.join("c/src/lib.rs"), "");

        let sc = CrateScope::load(root);
        assert!(!sc.is_empty());
        // a depends on b → allowed.
        assert!(sc.allows("a/src/lib.rs", "b/src/lib.rs"));
        // a does NOT depend on c → blocked (the over-link case).
        assert!(!sc.allows("a/src/lib.rs", "c/src/lib.rs"));
        // same crate always allowed.
        assert!(sc.allows("a/src/lib.rs", "a/src/other.rs"));
    }

    #[test]
    fn empty_scope_allows_everything() {
        let tmp = TempDir::new().unwrap();
        let sc = CrateScope::load(tmp.path());
        assert!(sc.is_empty());
        assert!(sc.allows("x/a.py", "y/b.py"));
    }
}
