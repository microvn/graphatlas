//! S-005 ga_architecture — module map (Layer 4 Meta).
//!
//! Spec contract (graphatlas-v1.1-tools.md S-005):
//!   AS-014: Module map happy path — modules + inter-module edges
//!     weighted by call/import counts.
//!   AS-015: Architecture with depth limit — `max_modules` cap +
//!     `meta.truncated` + `meta.total_modules`.
//!   Tools-C6: `meta.convention_used` names which convention applied.
//!
//! Module discovery conventions:
//!   - **python-init-py** — directory containing `__init__.py`
//!   - **cargo**          — directory containing `Cargo.toml`
//!   - **node-package**   — directory containing `package.json`
//!
//! Module name = the directory's basename (e.g. `auth/__init__.py` → `auth`).
//! Files = every indexed file under the module's root, EXCLUDING files
//! that fall under a deeper module (so a sub-package owns its own files).
//!
//! Edges = inter-module aggregation of CALLS / IMPORTS / EXTENDS edges.
//! Self-loops (caller and callee in the same module) are dropped per
//! AS-014 intent — the map is for ORIENTATION, not intra-module analysis.

use crate::common::is_safe_ident;
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub files: Vec<String>,
    pub symbol_count: u32,
    pub public_api: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleEdge {
    pub from: String,
    pub to: String,
    pub weight: u32,
    /// `"calls"` | `"imports"` | `"extends"`.
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchitectureMeta {
    pub truncated: bool,
    pub total_modules: u32,
    /// Tools-C6: e.g. `"python-init-py"`, `"cargo"`, `"node-package"`,
    /// or a comma-joined mix `"python-init-py,cargo"` for polyglot repos.
    /// Always non-empty — `"none"` when the repo had no module markers.
    pub convention_used: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchitectureResponse {
    pub modules: Vec<Module>,
    pub edges: Vec<ModuleEdge>,
    pub meta: ArchitectureMeta,
}

#[derive(Debug, Clone, Default)]
pub struct ArchitectureRequest {
    /// Optional cap on the number of modules returned (top-N by
    /// `symbol_count`). `None` = no cap. `Some(0)` is rejected as
    /// `InvalidParams` — meaningless query.
    pub max_modules: Option<u32>,
}

/// Public-API list cap per module. Larger lists obscure the orientation
/// the tool is meant to provide; the LLM can still drill down via
/// `ga_symbols`.
const PUBLIC_API_CAP: usize = 10;

// ─────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────

pub fn architecture(store: &Store, req: &ArchitectureRequest) -> Result<ArchitectureResponse> {
    if matches!(req.max_modules, Some(0)) {
        return Err(Error::InvalidParams(
            "ga_architecture: `max_modules` must be ≥ 1 (or omitted for no cap)".to_string(),
        ));
    }

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    if graph_is_empty(&conn)? {
        return Err(Error::IndexNotReady {
            status: "indexing".to_string(),
            progress: 0.0,
        });
    }

    let repo_root = PathBuf::from(store.metadata().repo_root.clone());

    // Git submodules are vendored third-party code, not this repo's — drop
    // their module markers and files so they don't pollute the orientation
    // map (e.g. `benches/fixtures/*` would otherwise dominate it).
    let excluded = read_submodule_paths(&repo_root);
    let raw: Vec<RawModule> = discover_modules(&repo_root)
        .into_iter()
        .filter(|m| !is_under_any(&m.root, &excluded))
        .collect();
    let convention_used = describe_conventions(&raw);

    let indexed_files: Vec<String> = list_indexed_files(&conn)?
        .into_iter()
        .filter(|f| !is_under_any(f, &excluded))
        .collect();
    let mut modules = build_modules(&raw, &indexed_files, &repo_root);
    populate_symbol_counts(&conn, &mut modules)?;

    let total_modules = modules.len() as u32;
    let cap = req.max_modules.map(|n| n as usize);
    let truncated = cap.map(|n| modules.len() > n).unwrap_or(false);
    if let Some(n) = cap {
        modules.sort_by(|a, b| {
            b.symbol_count
                .cmp(&a.symbol_count)
                .then(a.name.cmp(&b.name))
        });
        modules.truncate(n);
    } else {
        modules.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let edges = compute_edges(&conn, &raw, &modules)?;

    Ok(ArchitectureResponse {
        modules,
        edges,
        meta: ArchitectureMeta {
            truncated,
            total_modules,
            convention_used,
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Graph utilities
// ─────────────────────────────────────────────────────────────────────────

fn graph_is_empty(conn: &lbug::Connection<'_>) -> Result<bool> {
    let rs = conn
        .query("MATCH (s:Symbol) RETURN count(s)")
        .map_err(|e| Error::Other(anyhow::anyhow!("architecture count: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n == 0);
        }
    }
    Ok(true)
}

fn list_indexed_files(conn: &lbug::Connection<'_>) -> Result<Vec<String>> {
    let rs = conn
        .query("MATCH (f:File) RETURN f.path")
        .map_err(|e| Error::Other(anyhow::anyhow!("architecture files: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(p)) = row.into_iter().next() {
            out.push(p);
        }
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────
// Module discovery (Tools-C6)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RawModule {
    name: String,
    /// Repo-relative module root directory ("" for repo-root module).
    root: String,
    convention: &'static str,
}

fn discover_modules(repo_root: &Path) -> Vec<RawModule> {
    let mut out: Vec<RawModule> = Vec::new();
    walk_dirs(repo_root, &mut |abs_dir, rel_dir| {
        if has_marker(abs_dir, "__init__.py") {
            out.push(RawModule {
                name: dir_basename(rel_dir),
                root: rel_dir.to_string(),
                convention: "python-init-py",
            });
        }
        if has_marker(abs_dir, "Cargo.toml") {
            out.push(RawModule {
                name: dir_basename(rel_dir),
                root: rel_dir.to_string(),
                convention: "cargo",
            });
        }
        if has_marker(abs_dir, "package.json") {
            out.push(RawModule {
                name: dir_basename(rel_dir),
                root: rel_dir.to_string(),
                convention: "node-package",
            });
        }
    });
    // Drop duplicates: same root + same convention. Different conventions
    // on the same dir (e.g. a polyglot package) yield separate entries.
    out.sort_by(|a, b| {
        a.root
            .cmp(&b.root)
            .then(a.convention.cmp(b.convention))
            .then(a.name.cmp(&b.name))
    });
    out.dedup_by(|a, b| a.root == b.root && a.convention == b.convention);
    out
}

fn dir_basename(rel_dir: &str) -> String {
    if rel_dir.is_empty() {
        return "(root)".to_string();
    }
    rel_dir.rsplit('/').next().unwrap_or(rel_dir).to_string()
}

fn has_marker(dir: &Path, marker: &str) -> bool {
    dir.join(marker).is_file()
}

/// Per-module display names, unique across the set. A basename used by exactly
/// one module stays as-is; a basename shared by ≥2 modules is replaced with the
/// module's (unique) root path so the two never collapse into one identity.
/// Roots are unique by construction (discover_modules dedups on root).
fn assign_unique_names(raw: &[RawModule]) -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for m in raw {
        *counts.entry(m.name.as_str()).or_insert(0) += 1;
    }
    raw.iter()
        .map(|m| {
            let collides = counts.get(m.name.as_str()).copied().unwrap_or(0) > 1;
            if collides && !m.root.is_empty() {
                m.root.clone()
            } else {
                m.name.clone()
            }
        })
        .collect()
}

/// Repo-relative submodule paths declared in `.gitmodules` (git's `path = …`
/// lines). These subtrees are vendored code, not this repo's, and must not
/// appear in the module map. Empty when there is no `.gitmodules`.
fn read_submodule_paths(repo_root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(repo_root.join(".gitmodules")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("path") else {
            continue;
        };
        let Some(val) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let p = val.trim().trim_end_matches('/').replace('\\', "/");
        if !p.is_empty() {
            out.push(p);
        }
    }
    out
}

/// True when `rel` is one of, or lives under, any of the given repo-relative
/// prefixes. Used to drop git-submodule subtrees from discovery + file lists.
fn is_under_any(rel: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|p| rel == p || rel.starts_with(&format!("{p}/")))
}

fn describe_conventions(raw: &[RawModule]) -> String {
    if raw.is_empty() {
        return "none".to_string();
    }
    let mut set: BTreeSet<&'static str> = BTreeSet::new();
    for m in raw {
        set.insert(m.convention);
    }
    set.into_iter().collect::<Vec<_>>().join(",")
}

// ─────────────────────────────────────────────────────────────────────────
// Module materialisation
// ─────────────────────────────────────────────────────────────────────────

fn build_modules(raw: &[RawModule], indexed_files: &[String], repo_root: &Path) -> Vec<Module> {
    // Sort module roots longest-first so a sub-package claims its own
    // files before the parent sees them.
    let mut roots_sorted: Vec<&RawModule> = raw.iter().collect();
    roots_sorted.sort_by(|a, b| b.root.len().cmp(&a.root.len()));

    // Two modules can share a basename (e.g. `svc/common` + `lib/common`).
    // Keying identity on the basename collapses them in `file_to_module` /
    // `live_names` and conflates their symbol counts and edges. Disambiguate
    // colliding basenames to their unique root path so each module stays a
    // distinct node in the map.
    let names = assign_unique_names(raw);

    let mut by_index: BTreeMap<usize, Module> = BTreeMap::new();
    for (i, _m) in raw.iter().enumerate() {
        by_index.insert(
            i,
            Module {
                name: names[i].clone(),
                files: Vec::new(),
                symbol_count: 0,
                public_api: Vec::new(),
            },
        );
    }

    for file in indexed_files {
        if let Some((idx, _)) = pick_owning_module(file, raw, &roots_sorted) {
            by_index.get_mut(&idx).unwrap().files.push(file.clone());
        }
    }

    // Public API: per-module Python `__all__` exports (re-uses the
    // single-line list/tuple parser shipped in S-003 dead_code).
    for (idx, m) in by_index.iter_mut() {
        if raw[*idx].convention == "python-init-py" {
            let init_path = if raw[*idx].root.is_empty() {
                "__init__.py".to_string()
            } else {
                format!("{}/__init__.py", raw[*idx].root)
            };
            let abs = repo_root.join(&init_path);
            if let Ok(text) = std::fs::read_to_string(&abs) {
                let mut names = parse_dunder_all(&text);
                names.retain(|n| is_safe_ident(n));
                names.truncate(PUBLIC_API_CAP);
                m.public_api = names;
            }
        }
        m.files.sort();
    }

    by_index.into_values().collect()
}

/// Pick the deepest module whose root prefix matches the file path.
/// Returns `(raw_index, RawModule)`. None if no module owns the file.
fn pick_owning_module<'a>(
    file: &str,
    raw: &'a [RawModule],
    roots_sorted_longest_first: &[&'a RawModule],
) -> Option<(usize, &'a RawModule)> {
    for m in roots_sorted_longest_first {
        if m.root.is_empty() || file.starts_with(&format!("{}/", m.root)) || file == m.root {
            // Find original index.
            for (i, candidate) in raw.iter().enumerate() {
                if candidate.root == m.root && candidate.convention == m.convention {
                    return Some((i, m));
                }
            }
        }
    }
    None
}

fn populate_symbol_counts(conn: &lbug::Connection<'_>, modules: &mut [Module]) -> Result<()> {
    if modules.is_empty() {
        return Ok(());
    }
    // Pull symbol → file once, then bucket by module's file list.
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.file")
        .map_err(|e| Error::Other(anyhow::anyhow!("architecture symbols: {e}")))?;
    let mut per_file: BTreeMap<String, u32> = BTreeMap::new();
    for row in rs {
        if let Some(lbug::Value::String(p)) = row.into_iter().next() {
            *per_file.entry(p).or_insert(0) += 1;
        }
    }
    for m in modules.iter_mut() {
        let mut total = 0u32;
        for f in &m.files {
            total += per_file.get(f).copied().unwrap_or(0);
        }
        m.symbol_count = total;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Inter-module edge computation
// ─────────────────────────────────────────────────────────────────────────

fn compute_edges(
    conn: &lbug::Connection<'_>,
    raw: &[RawModule],
    modules: &[Module],
) -> Result<Vec<ModuleEdge>> {
    if modules.is_empty() {
        return Ok(Vec::new());
    }
    // Build file → module name map (only for retained modules).
    let live_names: BTreeSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    let mut file_to_module: BTreeMap<String, String> = BTreeMap::new();
    let mut roots_sorted: Vec<&RawModule> = raw.iter().collect();
    roots_sorted.sort_by(|a, b| b.root.len().cmp(&a.root.len()));
    for m in modules {
        for f in &m.files {
            file_to_module.insert(f.clone(), m.name.clone());
        }
    }
    let _ = roots_sorted; // map already populated by caller's `files`.

    let mut counts: BTreeMap<(String, String, &'static str), u32> = BTreeMap::new();

    // Map a (from_file, to_file) pair to a live inter-module (from, to), or
    // None for intra-module / dropped-module / unowned files.
    let resolve_pair = |from_file: &str, to_file: &str| -> Option<(String, String)> {
        let from_mod = file_to_module.get(from_file)?;
        let to_mod = file_to_module.get(to_file)?;
        if from_mod == to_mod {
            return None; // intra-module — not in scope
        }
        if !live_names.contains(from_mod.as_str()) || !live_names.contains(to_mod.as_str()) {
            return None;
        }
        Some((from_mod.clone(), to_mod.clone()))
    };

    let read_pair = |cols: Vec<lbug::Value>| -> Option<(String, String)> {
        match (cols.first(), cols.get(1)) {
            (Some(lbug::Value::String(a)), Some(lbug::Value::String(b))) => {
                Some((a.clone(), b.clone()))
            }
            _ => None,
        }
    };

    // calls / extends — row-counted: each call / reference / extends site adds
    // weight to the module-pair coupling.
    let edge_specs: &[(&str, &str)] = &[
        (
            "MATCH (caller:Symbol)-[:CALLS]->(callee:Symbol) RETURN caller.file, callee.file",
            "calls",
        ),
        (
            "MATCH (caller:Symbol)-[:REFERENCES]->(target:Symbol) RETURN caller.file, target.file",
            "calls",
        ),
        (
            "MATCH (sub:Symbol)-[:EXTENDS]->(base:Symbol) RETURN sub.file, base.file",
            "extends",
        ),
    ];
    for (cypher, kind) in edge_specs {
        let rs = conn
            .query(cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("architecture edges {kind}: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            let Some((from_file, to_file)) = read_pair(cols) else {
                continue;
            };
            if let Some((from_mod, to_mod)) = resolve_pair(&from_file, &to_file) {
                *counts.entry((from_mod, to_mod, kind)).or_insert(0) += 1;
            }
        }
    }

    // imports — counted as DISTINCT file-pairs (file_pair_count semantics).
    // Union two sources so non-Python langs get an imports dimension:
    //   (a) File-[:IMPORTS]->File   — Python / TS path-resolved imports.
    //   (b) File-[:IMPORTS_NAMED]->Symbol (→ Symbol.file) — langs whose path
    //       resolver is deferred (e.g. Rust) but whose imports the indexer
    //       resolved via the name-fallback. Without this the imports edge set
    //       is silently empty for every non-Python/TS repo.
    // Dedup over (src_file, dst_file) so a pair present in both tables (Python)
    // counts once — preserving the existing file-pair weight semantics.
    let mut import_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for cypher in [
        "MATCH (src:File)-[:IMPORTS]->(dst:File) RETURN src.path, dst.path",
        "MATCH (f:File)-[:IMPORTS_NAMED]->(s:Symbol) RETURN f.path, s.file",
    ] {
        let rs = conn
            .query(cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("architecture imports: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if let Some(pair) = read_pair(cols) {
                import_pairs.insert(pair);
            }
        }
    }
    for (from_file, to_file) in &import_pairs {
        if let Some((from_mod, to_mod)) = resolve_pair(from_file, to_file) {
            *counts.entry((from_mod, to_mod, "imports")).or_insert(0) += 1;
        }
    }

    let mut edges: Vec<ModuleEdge> = counts
        .into_iter()
        .map(|((from, to, kind), weight)| ModuleEdge {
            from,
            to,
            weight,
            kind: kind.to_string(),
        })
        .collect();
    edges.sort_by(|a, b| {
        a.from
            .cmp(&b.from)
            .then(a.to.cmp(&b.to))
            .then(a.kind.cmp(&b.kind))
    });
    Ok(edges)
}

// ─────────────────────────────────────────────────────────────────────────
// `__all__` parser — duplicated locally rather than reaching into
// dead_code.rs (its helper is `pub(crate)`-style and the LoC carve-out
// shelf already includes a refactor to share it).
// ─────────────────────────────────────────────────────────────────────────

fn parse_dunder_all(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let after_eq = match trimmed.strip_prefix("__all__") {
            Some(rest) => rest.trim_start().strip_prefix('=').map(str::trim_start),
            None => None,
        };
        let Some(rhs) = after_eq else { continue };
        let inner = if let Some(s) = rhs
            .strip_prefix('[')
            .and_then(|s| s.find(']').map(|i| &s[..i]))
        {
            s
        } else if let Some(s) = rhs
            .strip_prefix('(')
            .and_then(|s| s.find(')').map(|i| &s[..i]))
        {
            s
        } else {
            continue;
        };
        for raw in inner.split(',') {
            let token = raw.trim().trim_matches(|c: char| c == '"' || c == '\'');
            if !token.is_empty() {
                out.push(token.to_string());
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// Bounded directory walk (skip hidden + common build dirs).
// ─────────────────────────────────────────────────────────────────────────

fn walk_dirs<F: FnMut(&Path, &str)>(repo_root: &Path, on_dir: &mut F) {
    if !repo_root.exists() {
        return;
    }
    fn skip(name: &str) -> bool {
        matches!(
            name,
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | "__pycache__"
                | ".graphatlas"
        )
    }
    let mut stack: Vec<PathBuf> = vec![repo_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rel = dir
            .strip_prefix(repo_root)
            .unwrap_or(&dir)
            .to_string_lossy()
            .replace('\\', "/");
        on_dir(&dir, rel.as_ref());
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || skip(&name_str) {
                continue;
            }
            stack.push(entry.path());
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn dir_basename_handles_root() {
        assert_eq!(dir_basename(""), "(root)");
        assert_eq!(dir_basename("auth"), "auth");
        assert_eq!(dir_basename("svc/auth"), "auth");
    }

    #[test]
    fn parse_dunder_all_single_line_list() {
        let names = parse_dunder_all("__all__ = ['foo', 'bar']\n");
        assert_eq!(names, vec!["foo", "bar"]);
    }

    #[test]
    fn describe_conventions_polyglot_sorted() {
        let raw = vec![
            RawModule {
                name: "a".into(),
                root: "a".into(),
                convention: "python-init-py",
            },
            RawModule {
                name: "b".into(),
                root: "b".into(),
                convention: "cargo",
            },
        ];
        assert_eq!(describe_conventions(&raw), "cargo,python-init-py");
    }

    #[test]
    fn describe_conventions_empty_returns_none() {
        assert_eq!(describe_conventions(&[]), "none");
    }

    #[test]
    fn assign_unique_names_disambiguates_basename_collision() {
        let raw = vec![
            RawModule {
                name: "common".into(),
                root: "svc/common".into(),
                convention: "python-init-py",
            },
            RawModule {
                name: "common".into(),
                root: "lib/common".into(),
                convention: "python-init-py",
            },
            RawModule {
                name: "solo".into(),
                root: "solo".into(),
                convention: "cargo",
            },
        ];
        let names = assign_unique_names(&raw);
        // Colliding basenames fall back to their unique root paths…
        assert_eq!(names[0], "svc/common");
        assert_eq!(names[1], "lib/common");
        // …a non-colliding basename keeps its short name.
        assert_eq!(names[2], "solo");
        // All names unique.
        let set: BTreeSet<&String> = names.iter().collect();
        assert_eq!(set.len(), names.len());
    }

    #[test]
    fn read_submodule_and_is_under_any() {
        assert!(is_under_any(
            "benches/fixtures/tokio/src/lib.rs",
            &["benches/fixtures/tokio".to_string()]
        ));
        assert!(is_under_any("vendor/lib", &["vendor/lib".to_string()]));
        assert!(!is_under_any("app/main.py", &["vendor/lib".to_string()]));
        // root module ("") is never excluded.
        assert!(!is_under_any("", &["vendor/lib".to_string()]));
    }
}
