//! Ha-import-edge — `ga_architecture` GT rule for M3 bench (S-007).
//!
//! ## Anti-tautology policy
//! This rule does NOT import `ga_query::{dead_code, callers, rename_safety,
//! architecture, risk, minimal_context}` analysis types. Edges + module sets
//! come from raw AST + filesystem scan:
//! - imports:  `ga_parser::extract_imports`
//! - modules:  marker-based (`__init__.py` / `Cargo.toml` / `package.json`)
//!
//! See spec §C1 + AS-019/AS-020.
//!
//! ## GT shape
//! Two kinds of tasks:
//! - `kind: "edge"` — `{ module_a, module_b, file_pair_count }`
//! - `kind: "module"` — `{ module, files: Vec<String> }` (diagnostic only)
//!
//! AS-020 tautology-divergence (Spearman/F1 ≥ 0.95 on ≥4/5 fixtures →
//! TAUTOLOGICAL row) is enforced by the runner aggregator, not the rule.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::imports::extract_imports;
use ga_parser::walk::walk_repo;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct HaImportEdge;

impl Default for HaImportEdge {
    fn default() -> Self {
        Self
    }
}

impl GtRule for HaImportEdge {
    fn id(&self) -> &str {
        "Ha-import-edge"
    }
    fn uc(&self) -> &str {
        "architecture"
    }
    fn policy_bias(&self) -> &str {
        "Ha-import-edge — primary metric: F1 on edge-pairs (Spearman rank \
         correlation utility not yet wired; promoted from EXP follow-up in \
         Phase 3). Marker-based module GT (kind=module) is tautological-by-design \
         vs ga_architecture::discover_modules — tracked as DIAGNOSTIC only, \
         never as the primary spec gate. Module = dir-basename of nearest \
         ancestor with `__init__.py` / `Cargo.toml` / `package.json` marker \
         (mirrors ga_architecture::dir_basename for definitional alignment per \
         spec C2). Files outside any marked dir are dropped. Cross-module edge \
         is `(module_of_importer, module_of_imported)`; self-edges excluded. \
         Import target → owning module via root-prefix match (longest wins) \
         on the import's parsed target_path; unresolved imports (external \
         packages, relative paths) are dropped."
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // Pass 0 — discover marker-based modules. Mirrors
        // ga_query::architecture::discover_modules so bench module identity
        // aligns with the tool's per spec C2.
        let modules = discover_modules(fixture_dir);
        if modules.is_empty() {
            return Ok(Vec::new());
        }
        // Sort longest-root-first so deeper submodules win the ownership
        // contest (matches ga_architecture::pick_owning_module).
        let mut roots_sorted: Vec<&Module> = modules.iter().collect();
        roots_sorted.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
        let module_names: BTreeSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        // go.mod module prefix (independent of the engine — raw file read) for
        // Go import resolution. None for non-Go repos.
        let go_prefix = go_module_prefix(fixture_dir);

        let mut edges_by_pair: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        let mut files_by_module: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for entry in &report.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            let owner = match pick_owning_module(&rel, &roots_sorted) {
                Some(m) => m,
                None => continue, // file outside any marked dir — drop
            };
            files_by_module
                .entry(owner.name.clone())
                .or_default()
                .insert(rel.clone());

            let bytes = match std::fs::read(&entry.abs_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let imports = match extract_imports(entry.lang, &bytes) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for imp in imports {
                let target_module = resolve_target_module(
                    entry.lang,
                    &imp.target_path,
                    go_prefix.as_deref(),
                    &roots_sorted,
                );
                let Some(target) = target_module else {
                    continue;
                };
                if target == owner.name {
                    continue; // self-edge — same module, skip per policy_bias
                }
                if !module_names.contains(target.as_str()) {
                    continue;
                }
                edges_by_pair
                    .entry((owner.name.clone(), target))
                    .or_default()
                    .insert(rel.clone());
            }
        }

        // C1/C2 — Rust workspaces: manifest-grounded inter-crate dependency
        // edges from `cargo metadata` (build-system authority, independent of
        // the engine). On Rust these are the architecture dependencies the
        // tool's CALLS/EXTENDS edges recover (Rust per-file import resolution is
        // Python-only). Keyed by member dir basename to match discover_modules.
        for (from, to) in crate::gt_gen::cargo_deps::workspace_member_deps(fixture_dir) {
            if from != to
                && module_names.contains(from.as_str())
                && module_names.contains(to.as_str())
            {
                edges_by_pair
                    .entry((from, to))
                    .or_default()
                    .insert("__cargo_manifest__".to_string());
            }
        }

        // C1/C2 — JS/TS workspaces: inter-package dependency edges from the
        // on-disk `package.json` manifests (package-manager authority,
        // independent of the engine). Monorepos declare intra-workspace deps in
        // `dependencies` / `peerDependencies` / `devDependencies`; these are the
        // architecture dependencies the tool's IMPORTS edges recover once bare
        // workspace specifiers (`@scope/pkg`) resolve. Keyed by package dir
        // basename (root → `(root)`) to match discover_modules.
        for (from, to) in crate::gt_gen::package_deps::workspace_member_deps(fixture_dir) {
            if from != to
                && module_names.contains(from.as_str())
                && module_names.contains(to.as_str())
            {
                edges_by_pair
                    .entry((from, to))
                    .or_default()
                    .insert("__package_manifest__".to_string());
            }
        }

        let marked_modules: BTreeSet<String> = modules.iter().map(|m| m.name.clone()).collect();

        let mut out: Vec<GeneratedTask> = Vec::new();

        for ((mod_a, mod_b), importers) in &edges_by_pair {
            let task_id = format!("ha-edge::{mod_a}->{mod_b}");
            let importer_list: Vec<String> = importers.iter().cloned().collect();
            let query = json!({
                "kind": "edge",
                "module_a": mod_a,
                "module_b": mod_b,
                "file_pair_count": importer_list.len(),
                "importers": importer_list,
            });
            let expected = vec![format!("{mod_a}->{mod_b}={}", importers.len())];
            out.push(GeneratedTask {
                task_id,
                query,
                expected,
                rule: "Ha-import-edge".to_string(),
                rationale: format!(
                    "{} importing file(s) in `{mod_a}` reach into `{mod_b}`",
                    importers.len()
                ),
            });
        }

        for module in &marked_modules {
            let files: Vec<String> = files_by_module
                .get(module)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let task_id = format!("ha-module::{module}");
            let query = json!({
                "kind": "module",
                "module": module,
                "files": files,
            });
            let expected: Vec<String> = query
                .get("files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.push(GeneratedTask {
                task_id,
                query,
                expected,
                rule: "Ha-import-edge".to_string(),
                rationale: format!("marker-based module `{module}` (diagnostic, tautological)"),
            });
        }

        Ok(out)
    }
}

/// Marker-based module — mirrors `ga_query::architecture::RawModule`.
#[derive(Debug, Clone)]
struct Module {
    name: String,
    /// Relative root path from fixture root (e.g. `django/contrib/auth`).
    root: String,
}

/// Walk fixture; for each dir containing `__init__.py` / `Cargo.toml` /
/// `package.json`, emit a Module. Definitionally aligned with
/// `ga_query::architecture::discover_modules`.
fn discover_modules(fixture_dir: &Path) -> Vec<Module> {
    let mut out = Vec::new();
    // JS/TS workspace scope: restrict `package.json` modules to `workspaces`
    // glob members (+ root), aligning the GT module set with
    // `package_deps` + `ga_query::architecture` so non-member example apps
    // (`integration/`) are not nodes. Empty globs → every package.json dir, as
    // before (Rust/Python unaffected — they have no `workspaces`).
    let ws_globs = crate::gt_gen::package_deps::workspace_globs(fixture_dir);
    walk_dirs(fixture_dir, fixture_dir, &ws_globs, &mut out);
    // Dedup on root path (a polyglot dir may have both Cargo.toml +
    // package.json — emit once per root, last-write-wins on name; matches
    // `ga_architecture` which de-duplicates after the fact).
    out.sort_by(|a, b| a.root.cmp(&b.root));
    out.dedup_by(|a, b| a.root == b.root);
    // Mirror `ga_query::architecture::assign_unique_names` (C1 structural
    // mirror, not an analysis-type import): a basename shared by ≥2 modules is
    // disambiguated to its unique root path, so GT module identity matches the
    // engine's. Without this, same-basename dirs (a Django project's many
    // `migrations`/`tests`/`models` app dirs) collapse here while the engine
    // disambiguates them → node names disagree → recall craters.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in &out {
        *counts.entry(m.name.clone()).or_insert(0) += 1;
    }
    for m in &mut out {
        if counts.get(&m.name).copied().unwrap_or(0) > 1 && !m.root.is_empty() {
            m.name = m.root.clone();
        }
    }
    out
}

fn walk_dirs(repo_root: &Path, dir: &Path, ws_globs: &[String], out: &mut Vec<Module>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let rel = dir
        .strip_prefix(repo_root)
        .unwrap_or(dir)
        .to_string_lossy()
        .into_owned();
    let mk = |rel: &str| -> Module {
        let name = if rel.is_empty() {
            "(root)".to_string()
        } else {
            rel.rsplit('/').next().unwrap_or(rel).to_string()
        };
        Module {
            name,
            root: rel.to_string(),
        }
    };
    let mut pushed = false;
    for marker in ["__init__.py", "Cargo.toml", "package.json"] {
        if dir.join(marker).is_file() {
            // node-package modules outside the workspace glob (integration /
            // example apps) are leaf consumers, not architecture modules.
            if marker == "package.json" && !crate::gt_gen::package_deps::is_member(ws_globs, &rel) {
                break;
            }
            out.push(mk(&rel));
            pushed = true;
            break;
        }
    }
    // Go package = a directory holding `.go` files (no per-dir manifest).
    if !pushed && has_go_files(dir) {
        out.push(mk(&rel));
    }
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            // Skip vcs/node_modules/etc. — use the same conservative list
            // as ga_parser::walk.
            let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                basename,
                ".git" | "node_modules" | "target" | "dist" | "build" | ".tox" | "__pycache__"
            ) {
                continue;
            }
            walk_dirs(repo_root, &path, ws_globs, out);
        }
    }
}

fn pick_owning_module<'a>(rel_file: &str, roots_sorted: &[&'a Module]) -> Option<&'a Module> {
    for m in roots_sorted {
        if m.root.is_empty() || rel_file.starts_with(&format!("{}/", m.root)) || rel_file == m.root
        {
            return Some(m);
        }
    }
    None
}

/// Try to resolve an import target to one of the discovered modules.
///
/// SOUND ONLY for languages whose raw import maps to the source directory tree
/// without inventing edges:
/// - **Python** — dotted module path (`pkg.sub.foo` → `pkg/sub/foo`).
/// - **Go** — `<go.mod module prefix>/<pkg dir>` → strip the prefix, the rest
///   IS the package dir (`github.com/x/y/binding` → `binding`); bare prefix →
///   the root package. `None` for stdlib / third-party (no prefix match).
///
/// Rust `crate::x` / TS bare specifiers are NOT dir-tree paths without a
/// build manifest — those use the dedicated cargo/package authorities elsewhere
/// and return `None` here (no fabricated edge).
fn resolve_target_module(
    lang: ga_core::Lang,
    target_path: &str,
    go_prefix: Option<&str>,
    roots_sorted: &[&Module],
) -> Option<String> {
    if target_path.is_empty() {
        return None;
    }
    let candidates: Vec<String> = match lang {
        ga_core::Lang::Python => {
            if target_path.starts_with("./") || target_path.starts_with("../") {
                return None; // relative — likely same module
            }
            candidate_paths(target_path)
        }
        ga_core::Lang::Go => {
            let prefix = go_prefix?;
            let dir = if target_path == prefix {
                String::new() // bare module path → root package
            } else {
                target_path.strip_prefix(&format!("{prefix}/"))?.to_string()
            };
            vec![dir]
        }
        _ => return None,
    };
    for cand in &candidates {
        for m in roots_sorted {
            // Go root-package import (`cand` empty) → the `(root)` module.
            if cand.is_empty() {
                if m.root.is_empty() {
                    return Some(m.name.clone());
                }
                continue;
            }
            if m.root.is_empty() {
                continue;
            }
            if cand == &m.root || cand.starts_with(&format!("{}/", m.root)) {
                return Some(m.name.clone());
            }
        }
    }
    None
}

/// A Go package is any directory holding at least one `.go` source file.
fn has_go_files(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten()
        .any(|e| e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("go"))
}

/// `module <path>` directive from `go.mod` (raw read — C1 independent of the
/// engine). None when absent.
fn go_module_prefix(repo_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(repo_root.join("go.mod")).ok()?;
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("module ")
            .map(|m| m.trim().to_string())
    })
}

fn candidate_paths(target_path: &str) -> Vec<String> {
    let normalised = if target_path.contains("::") {
        target_path.replace("::", "/")
    } else if target_path.contains('.') && !target_path.contains('/') {
        target_path.replace('.', "/")
    } else {
        target_path.to_string()
    };
    let mut out = vec![normalised.clone()];
    // Also try progressively-shorter prefixes so `pkg.sub.foo` can match
    // a module rooted at `pkg/sub` (drop the leaf).
    let mut cur = normalised.as_str();
    while let Some((head, _)) = cur.rsplit_once('/') {
        if head.is_empty() {
            break;
        }
        out.push(head.to_string());
        cur = head;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ga_core::Lang;

    fn module(name: &str, root: &str) -> Module {
        Module {
            name: name.to_string(),
            root: root.to_string(),
        }
    }

    // Part A — the GT must only resolve import targets where the raw import
    // syntax soundly maps to the directory tree (Python dotted paths). It must
    // NOT fabricate edges for languages whose import path is not a dir-tree
    // path without manifest authority (Rust `::`, Go URL, TS bare specifier).

    #[test]
    fn python_dotted_import_resolves_to_module() {
        let mods = [module("auth", "svc/auth")];
        let roots: Vec<&Module> = mods.iter().collect();
        assert_eq!(
            resolve_target_module(Lang::Python, "svc.auth.login", None, &roots),
            Some("auth".to_string()),
            "Python dotted import maps to the dir tree — sound, must resolve"
        );
    }

    #[test]
    fn rust_path_import_is_not_fabricated() {
        // `use crate::sync::mpsc` must NOT path-prefix-match a module; Rust
        // module paths are not the file dir tree without mod/Cargo resolution.
        let mods = [module("sync", "crate/sync")];
        let roots: Vec<&Module> = mods.iter().collect();
        assert_eq!(
            resolve_target_module(Lang::Rust, "crate::sync::mpsc", None, &roots),
            None,
            "Rust :: import has no path-tree authority — must not be fabricated"
        );
    }

    #[test]
    fn go_import_without_module_prefix_is_not_fabricated() {
        // No go.mod prefix → a Go URL import cannot be resolved (don't invent).
        let mods = [module("gin", "gin")];
        let roots: Vec<&Module> = mods.iter().collect();
        assert_eq!(
            resolve_target_module(Lang::Go, "github.com/gin-gonic/gin/binding", None, &roots),
            None,
            "Go import without go.mod authority must not be fabricated"
        );
    }

    #[test]
    fn go_import_strips_module_prefix_to_package_dir() {
        // With the go.mod prefix, the path after it IS the package dir.
        let mods = [module("binding", "binding"), module("(root)", "")];
        let roots: Vec<&Module> = mods.iter().collect();
        let prefix = Some("github.com/gin-gonic/gin");
        assert_eq!(
            resolve_target_module(Lang::Go, "github.com/gin-gonic/gin/binding", prefix, &roots),
            Some("binding".to_string()),
            "Go import strips the module prefix → package dir"
        );
        assert_eq!(
            resolve_target_module(Lang::Go, "github.com/gin-gonic/gin", prefix, &roots),
            Some("(root)".to_string()),
            "bare module path → the root package"
        );
        assert_eq!(
            resolve_target_module(Lang::Go, "fmt", prefix, &roots),
            None,
            "stdlib import (no prefix match) → None"
        );
    }
}
