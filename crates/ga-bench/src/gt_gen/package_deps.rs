//! C1/C2 — npm/workspace package-dependency authority for the `architecture`
//! GT on TS/JS monorepos (analog of [`super::cargo_deps`] for Cargo).
//!
//! ## Anti-tautology policy (§C1)
//! Independent of the engine — does NOT import `ga_query::*` analysis types.
//! Authority is the on-disk `package.json` manifests (the package manager's own
//! declared graph), parsed directly. A workspace package's dependency on
//! ANOTHER workspace package (matched by the dependency name against the set of
//! in-repo `package.json` `name` fields) is the architectural edge.
//!
//! ## Scope (option B)
//! The set of "workspace packages" is the root `package.json` `workspaces` glob
//! (`packages/*`): only matching dirs (+ the root) count, so `integration/` /
//! example apps carrying a package.json are excluded — they are leaf consumers,
//! not architecture modules. When no `workspaces` field is declared (convention
//! monorepos), every package.json dir is a candidate member.
//!
//! Monorepos declare intra-workspace edges in `dependencies` /
//! `peerDependencies` / `devDependencies` (nest: `@nestjs/core` peer-depends
//! `@nestjs/common`); all three are read, then filtered to the member set.
//!
//! Edges are keyed by package DIRECTORY BASENAME (root package → `(root)`) to
//! match `ha_import_edge::discover_modules` / `ga_query::architecture`.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Deserialize, Default)]
struct PkgJson {
    name: Option<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, serde_json::Value>,
    #[serde(default, rename = "peerDependencies")]
    peer_dependencies: BTreeMap<String, serde_json::Value>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: BTreeMap<String, serde_json::Value>,
}

impl PkgJson {
    /// Union of all declared dependency names across the three sections.
    fn dep_names(&self) -> BTreeSet<String> {
        self.dependencies
            .keys()
            .chain(self.peer_dependencies.keys())
            .chain(self.dev_dependencies.keys())
            .cloned()
            .collect()
    }
}

/// Directed inter-package dependency edges `(from_module, to_module)` for a
/// JS/TS workspace, keyed by package dir basename (root → `(root)`).
///
/// Returns empty when no member package declares a dependency on another member
/// — single-package repos (radash) legitimately have no inter-module edges and
/// stay SKIPPED rather than fabricated.
pub fn workspace_member_deps(root: &Path) -> Vec<(String, String)> {
    let globs = workspace_globs(root);

    let mut manifests: Vec<(String, PkgJson)> = Vec::new(); // (module_id, parsed)
    collect_manifests(root, root, &globs, &mut manifests);

    // name → module id (members only): `@nestjs/common` → `common`, `preact`
    // → `(root)`.
    let mut name_to_module: BTreeMap<String, String> = BTreeMap::new();
    for (module_id, pkg) in &manifests {
        if let Some(name) = &pkg.name {
            name_to_module.insert(name.clone(), module_id.clone());
        }
    }

    let mut edges: Vec<(String, String)> = Vec::new();
    for (from, pkg) in &manifests {
        for dep in pkg.dep_names() {
            if let Some(to) = name_to_module.get(&dep) {
                if to != from {
                    edges.push((from.clone(), to.clone()));
                }
            }
        }
    }
    edges.sort();
    edges.dedup();
    edges
}

/// Workspace package globs from the root `package.json` `workspaces` field
/// (array form or `{packages: [...]}`). Empty when not declared.
pub(crate) fn workspace_globs(root: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let ws = match v.get("workspaces") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(serde_json::Value::Object(o)) => o
            .get("packages")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    ws.iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

/// Is repo-relative dir `rel` a workspace member? Root is always a member; with
/// no globs declared every package.json dir is; otherwise it must match a glob.
pub(crate) fn is_member(globs: &[String], rel: &str) -> bool {
    if rel.is_empty() || globs.is_empty() {
        return true;
    }
    for g in globs {
        let g = g.trim_end_matches('/');
        if let Some(prefix) = g.strip_suffix("/*").or_else(|| g.strip_suffix("/**")) {
            if let Some(rest) = rel.strip_prefix(&format!("{prefix}/")) {
                if !rest.is_empty() {
                    return true;
                }
            }
        } else if g == rel {
            return true;
        }
    }
    false
}

/// Recursively collect `(module_id, PkgJson)` for member `package.json` dirs
/// outside `node_modules` / vcs / build dirs.
fn collect_manifests(
    repo_root: &Path,
    dir: &Path,
    globs: &[String],
    out: &mut Vec<(String, PkgJson)>,
) {
    let rel = dir
        .strip_prefix(repo_root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/");
    let manifest = dir.join("package.json");
    if manifest.is_file() && is_member(globs, &rel) {
        if let Ok(bytes) = std::fs::read(&manifest) {
            if let Ok(pkg) = serde_json::from_slice::<PkgJson>(&bytes) {
                let module_id = if rel.is_empty() {
                    "(root)".to_string()
                } else {
                    rel.rsplit('/').next().unwrap_or(&rel).to_string()
                };
                out.push((module_id, pkg));
            }
        }
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                basename,
                "node_modules" | ".git" | "dist" | "build" | "target" | ".tox" | "__pycache__"
            ) {
                continue;
            }
            collect_manifests(repo_root, &path, globs, out);
        }
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
    fn peer_dependency_on_workspace_package_is_an_edge() {
        // nest shape: packages/core peer-depends @scope/common (NOT in
        // `dependencies`). The edge must still surface.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("package.json"),
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        );
        write(
            &root.join("packages/common/package.json"),
            r#"{"name":"@scope/common","version":"1.0.0"}"#,
        );
        write(
            &root.join("packages/core/package.json"),
            r#"{"name":"@scope/core","peerDependencies":{"@scope/common":"*"}}"#,
        );

        let edges = workspace_member_deps(root);
        assert!(
            edges.contains(&("core".to_string(), "common".to_string())),
            "core peer-dep on common must be an edge, got {edges:?}"
        );
    }

    #[test]
    fn non_member_integration_app_is_excluded_by_workspace_glob() {
        // nest shape: an integration/ sample app peer-deps @scope/core but is
        // NOT under the `packages/*` workspace glob → it is not a member, so its
        // edges are excluded (the precision-killer this option fixes).
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("package.json"),
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        );
        write(
            &root.join("packages/core/package.json"),
            r#"{"name":"@scope/core"}"#,
        );
        write(
            &root.join("integration/sample/package.json"),
            r#"{"name":"sample","dependencies":{"@scope/core":"*"}}"#,
        );

        let edges = workspace_member_deps(root);
        assert!(
            !edges.iter().any(|(f, _)| f == "sample"),
            "non-member integration app must not contribute edges, got {edges:?}"
        );
    }

    #[test]
    fn root_package_dependency_is_labelled_root() {
        // preact shape (no workspaces glob): compat peer-deps the root `preact`.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("package.json"),
            r#"{"name":"preact","version":"10.0.0"}"#,
        );
        write(
            &root.join("compat/package.json"),
            r#"{"name":"preact-compat","version":"1.0.0","peerDependencies":{"preact":"^10"}}"#,
        );

        let edges = workspace_member_deps(root);
        assert!(
            edges.contains(&("compat".to_string(), "(root)".to_string())),
            "compat dep on root preact must be labelled (root), got {edges:?}"
        );
    }

    #[test]
    fn external_npm_dependency_is_not_an_edge() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("package.json"),
            r#"{"name":"solo","version":"1.0.0","dependencies":{"lodash":"^4"}}"#,
        );
        assert!(workspace_member_deps(root).is_empty());
    }
}
