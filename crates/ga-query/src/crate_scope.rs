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
    /// Rust-identifier crate name (`-` normalised to `_`, as written in a
    /// `use` path) → crate root dir. Lets an explicit `use foo_bar::Baz`
    /// resolve to the exact crate instead of a repo-wide name guess.
    name_to_dir: HashMap<String, String>,
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
        // Keep the workspace-root package's dir (empty rel-path) in `roots` so
        // `crate_of` can attribute top-level `src/*.rs` files to it. Without
        // this the root crate is unconstrained: its calls/imports resolve into
        // any sibling crate (root -> leaf) and any crate's name-fallback
        // resolves into root (leaf -> root), both unfilterable — the regex
        // over-link tail.
        // Rust `use` paths write the crate name with `-` → `_`; key the map in
        // that form so a `use regex_automata::…` segment resolves directly.
        let name_to_dir: HashMap<String, String> = name_dir
            .iter()
            .filter(|(_, dir)| !dir.is_empty())
            .map(|(name, dir)| (name.replace('-', "_"), dir.clone()))
            .collect();
        let mut roots: Vec<String> = name_dir.into_values().collect();
        roots.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        roots.dedup();
        CrateScope {
            roots,
            allowed,
            name_to_dir,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Resolve the leading segment of a Rust `use` path (e.g. `regex_automata`
    /// in `use regex_automata::dfa::DFA`) to the workspace crate's root dir.
    /// `None` for `crate` / `super` / `self` / `std` / external crates not in
    /// the workspace. This is the explicit-import authority: the `use` names
    /// the source crate, so resolution needs no repo-wide name guess.
    pub fn crate_dir_for_use_segment(&self, segment: &str) -> Option<&str> {
        self.name_to_dir.get(segment).map(String::as_str)
    }

    fn crate_of<'a>(&'a self, file: &str) -> Option<&'a str> {
        // Longest matching member dir wins (`roots` is sorted longest-first).
        for r in &self.roots {
            if r.is_empty() {
                continue;
            }
            if file == r.as_str() || file.starts_with(&format!("{r}/")) {
                return Some(r);
            }
        }
        // Not under any member subdir → the workspace-root crate (rel-path ""),
        // when the workspace root is itself a package.
        if self.roots.iter().any(String::is_empty) {
            Some("")
        } else {
            None
        }
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
    fn root_package_is_constrained_by_its_own_deps() {
        // regex shape: the workspace root manifest is itself a package whose
        // top-level src/ lives at rel-path "". It depends on `member` but not
        // on `other`. Edges in/out of the root crate must respect that.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\", \"other\"]\nresolver = \"2\"\n\n[package]\nname = \"top\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nmember = { path = \"member\" }\n",
        );
        write(&root.join("src/lib.rs"), "");
        write(
            &root.join("member/Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&root.join("member/src/lib.rs"), "");
        write(
            &root.join("other/Cargo.toml"),
            "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&root.join("other/src/lib.rs"), "");

        let sc = CrateScope::load(root);
        // root (src/lib.rs) depends on member → allowed.
        assert!(sc.allows("src/lib.rs", "member/src/lib.rs"));
        // root does NOT depend on other → blocked (root -> leaf over-link).
        assert!(!sc.allows("src/lib.rs", "other/src/lib.rs"));
        // other does NOT depend on root → blocked (leaf -> root over-link).
        assert!(!sc.allows("other/src/lib.rs", "src/lib.rs"));
    }

    #[test]
    fn empty_scope_allows_everything() {
        let tmp = TempDir::new().unwrap();
        let sc = CrateScope::load(tmp.path());
        assert!(sc.is_empty());
        assert!(sc.allows("x/a.py", "y/b.py"));
    }
}
