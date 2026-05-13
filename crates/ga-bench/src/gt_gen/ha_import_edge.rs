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
                let target_module = resolve_target_module(&imp.target_path, &roots_sorted);
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
    walk_dirs(fixture_dir, fixture_dir, &mut out);
    // Dedup on root path (a polyglot dir may have both Cargo.toml +
    // package.json — emit once per root, last-write-wins on name; matches
    // `ga_architecture` which de-duplicates after the fact).
    out.sort_by(|a, b| a.root.cmp(&b.root));
    out.dedup_by(|a, b| a.root == b.root);
    out
}

fn walk_dirs(repo_root: &Path, dir: &Path, out: &mut Vec<Module>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for marker in ["__init__.py", "Cargo.toml", "package.json"] {
        if dir.join(marker).is_file() {
            let rel = dir
                .strip_prefix(repo_root)
                .unwrap_or(dir)
                .to_string_lossy()
                .into_owned();
            let name = if rel.is_empty() {
                "(root)".to_string()
            } else {
                rel.rsplit('/').next().unwrap_or(&rel).to_string()
            };
            out.push(Module { name, root: rel });
            break;
        }
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
            walk_dirs(repo_root, &path, out);
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
/// Best-effort: convert Python `pkg.sub.foo` → candidate path `pkg/sub/foo`
/// and `pkg/sub`, see if any module's `root` matches a prefix; for Rust /
/// TS / Go just normalise the slash form.
fn resolve_target_module(target_path: &str, roots_sorted: &[&Module]) -> Option<String> {
    if target_path.is_empty() {
        return None;
    }
    if target_path.starts_with("./") || target_path.starts_with("../") {
        return None; // relative — likely same module
    }
    // Build slash-form candidate paths (longest-prefix-first).
    let candidates = candidate_paths(target_path);
    for cand in &candidates {
        for m in roots_sorted {
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
