//! C1/C2 — Cargo-workspace module-dependency authority for the `architecture`
//! GT (kind-agnostic module edges on Rust workspaces).
//!
//! ## Anti-tautology policy (§C1)
//! Independent of the engine — does NOT import `ga_query::*` analysis types.
//! Authority is `cargo metadata` (the build system's own resolved workspace),
//! not graphatlas. Using `cargo metadata` (not a hand-parse) makes workspace
//! globs, `name.workspace` inheritance, `[[bench]]`/`[[example]]` `name` decoys,
//! and `-`/`_` normalization all resolved correctly by cargo itself.
//!
//! Edges are keyed by member DIRECTORY BASENAME to match
//! `ga_query::architecture`'s module identity (dir basename), so GT edges and
//! engine edges compare on the same node names.

use serde::Deserialize;
use std::collections::BTreeMap;
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

/// Directed inter-member dependency edges `(from_basename, to_basename)`.
///
/// Returns empty when `root` is not a Cargo workspace, or `cargo metadata`
/// is unavailable / fails offline — the fixture then contributes no Rust
/// dependency GT (never fabricated).
pub fn workspace_member_deps(root: &Path) -> Vec<(String, String)> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(root)
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let meta: Metadata = match serde_json::from_slice(&stdout) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let mut name_to_base: BTreeMap<String, String> = BTreeMap::new();
    for p in &meta.packages {
        if let Some(base) = manifest_dir_basename(&p.manifest_path) {
            name_to_base.insert(p.name.clone(), base);
        }
    }

    let mut edges: Vec<(String, String)> = Vec::new();
    for p in &meta.packages {
        let Some(from) = manifest_dir_basename(&p.manifest_path) else {
            continue;
        };
        for d in &p.dependencies {
            if let Some(to) = name_to_base.get(&d.name) {
                if *to != from {
                    edges.push((from.clone(), to.clone()));
                }
            }
        }
    }
    edges.sort();
    edges.dedup();
    edges
}

/// `/abs/path/crates/ga-query/Cargo.toml` → `ga-query`.
fn manifest_dir_basename(manifest_path: &str) -> Option<String> {
    Path::new(manifest_path)
        .parent()?
        .file_name()?
        .to_str()
        .map(String::from)
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
    fn resolves_inter_member_dependency_edge() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
        );
        write(
            &root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nb = { path = \"../b\" }\n",
        );
        write(&root.join("a/src/lib.rs"), "");
        write(
            &root.join("b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&root.join("b/src/lib.rs"), "");

        let edges = workspace_member_deps(root);
        assert!(edges.contains(&("a".to_string(), "b".to_string())));
        assert!(!edges.contains(&("b".to_string(), "a".to_string())));
    }

    #[test]
    fn external_dependency_is_not_an_edge() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\"]\nresolver = \"2\"\n",
        );
        write(
            &root.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1\"\n",
        );
        write(&root.join("a/src/lib.rs"), "");

        assert!(workspace_member_deps(root).is_empty());
    }

    #[test]
    fn non_workspace_dir_yields_no_edges() {
        let tmp = TempDir::new().unwrap();
        assert!(workspace_member_deps(tmp.path()).is_empty());
    }
}
