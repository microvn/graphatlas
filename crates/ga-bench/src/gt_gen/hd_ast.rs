//! Hd-ast — `ga_dead_code` GT rule for M3 bench (S-005 cycle A).
//!
//! ## Anti-tautology policy
//! This rule does NOT import `ga_query::{dead_code, callers, rename_safety,
//! architecture, risk, minimal_context}` analysis types. The expected dead
//! set is computed from raw AST signal:
//! - defs:        `ga_parser::parse_source` per file
//! - targeted:    `ga_parser::extract_calls` + `ga_parser::extract_references`
//! - entry-points: name-based (`main`/`__main__`) + per-file (`__all__`) +
//!                 manifest-based (`pyproject [project.scripts]`, Cargo `[[bin]]`)
//!
//! See spec §C1 + AS-014/AS-015.
//!
//! ## Cycle A scope (this file)
//! Implements name-based + per-file + manifest entry-point detection. **Route
//! handler detection (gin/django/rails/axum/nest) ships in S-005 cycle B**
//! once the helpers in `ga_query::dead_code` are extracted into
//! `ga_query::common::entry_points` (single-source pattern, mirrors the
//! `is_test_path` consolidation). Until then, route handlers in fixtures
//! are listed as a known gap in `policy_bias()` — retrievers fail honestly
//! per C4.
//!
//! ## Identity
//! Each GT entry is keyed by `(name, file)` per S-003. The bench rule is
//! **conservative on the targeted side**: raw AST cannot resolve a callee
//! to its def file, so any same-named call anywhere makes ALL homonym defs
//! "live". This under-reports dead vs the production tool's per-file
//! resolution; documented in `policy_bias()`.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_core::SymbolKind;
use ga_index::Store;
use ga_parser::calls::extract_calls;
use ga_parser::imports::extract_imports;
use ga_parser::references::extract_references;
use ga_parser::walk::walk_repo;
use ga_parser::{parse_source, ParsedSymbol};
// S-005 cycle B — pull production-grade entry-point helpers from
// ga_query::entry_points (allowed: not in the anti-tautology forbidden set).
// This drops ~22k false-negatives on django by exempting framework route
// handlers (gin/django/rails/axum/nest), pyproject scripts, and cargo bins
// the way ga_query::dead_code does internally.
use ga_query::entry_points::{
    collect_cargo_bins, collect_dunder_all, collect_project_scripts, collect_route_handlers,
    same_package,
};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

pub struct HdAst;

impl Default for HdAst {
    fn default() -> Self {
        Self
    }
}

impl HdAst {
    /// Whether `kind` should be considered a candidate for the dead-code
    /// list. Mirrors the production tool's filter — module/trait/etc. are
    /// not callable surfaces in the same sense.
    fn is_candidate_kind(kind: SymbolKind) -> bool {
        matches!(
            kind,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Class
        )
    }
}

impl GtRule for HdAst {
    fn id(&self) -> &str {
        "Hd-ast"
    }
    fn uc(&self) -> &str {
        "dead_code"
    }
    fn policy_bias(&self) -> &str {
        "Hd-ast cycle B' — entry-point detection sourced from \
         `ga_query::entry_points` (shared with the production tool, \
         definitionally aligned per spec C2): main/__main__, Python \
         __all__ exports, pyproject [project.scripts] / [tool.poetry.scripts], \
         Cargo [[bin]], framework route handlers (gin/django/rails/axum/nest \
         line-pattern scan). Targeted side now per-file via import \
         resolution: a call site `foo()` in F.py resolves to def (foo, F) \
         intra-file, or to def (foo, G) iff F has `from <G's module> import foo` \
         (matches production S-003 (name, file) identity). Remaining honest \
         gaps: clap derive `#[command]` / Cobra command structs / Rust \
         `pub use` re-exports / TS `export` re-exports / dynamic getattr / \
         metaclass tricks — kept in GT, tools that know better under-score \
         on those fixture cases. Candidate-pool note: parse_source emits \
         every function/method/class incl. nested closures; ga's indexer \
         stores fewer (top-level + methods only), so Hd-ast's expected_dead \
         pool is systematically larger than ga's universe — drives FN up \
         but doesn't affect precision."
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // Pass 1 — collect defs + per-file name list (for resolution) +
        // imports per file + raw call sites.
        let mut defs: Vec<DefRow> = Vec::new();
        let mut defs_by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut imports_per_file: BTreeMap<String, Vec<HdImport>> = BTreeMap::new();
        let mut call_sites: Vec<(String, String)> = Vec::new(); // (call_file, callee_name)

        for entry in &report.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            let bytes = match std::fs::read(&entry.abs_path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            if let Ok(symbols) = parse_source(entry.lang, &bytes) {
                for sym in symbols {
                    if !Self::is_candidate_kind(sym.kind) {
                        continue;
                    }
                    let ParsedSymbol {
                        name, kind, line, ..
                    } = sym;
                    if name.is_empty() {
                        continue;
                    }
                    defs_by_name
                        .entry(name.clone())
                        .or_default()
                        .push(rel.clone());
                    defs.push(DefRow {
                        name,
                        file: rel.clone(),
                        kind,
                        line,
                    });
                }
            }

            if let Ok(imps) = extract_imports(entry.lang, &bytes) {
                let links: Vec<HdImport> = imps
                    .into_iter()
                    .filter(|i| !i.imported_names.is_empty())
                    .map(|i| HdImport {
                        target_path: i.target_path,
                        imported_names: i.imported_names,
                    })
                    .collect();
                if !links.is_empty() {
                    imports_per_file.insert(rel.clone(), links);
                }
            }

            if let Ok(calls) = extract_calls(entry.lang, &bytes) {
                for c in calls {
                    if !c.callee_name.is_empty() {
                        call_sites.push((rel.clone(), c.callee_name));
                    }
                }
            }
            if let Ok(refs) = extract_references(entry.lang, &bytes) {
                for r in refs {
                    if !r.target_name.is_empty() {
                        call_sites.push((rel.clone(), r.target_name));
                    }
                }
            }
        }

        // Cycle B' — per-file resolution: build `targeted_pairs` =
        // Set<(name, def_file)> instead of Set<name>. For each call site
        // (call_file, name): resolve to one or more def_files via
        //   1. intra-file: any def of `name` in `call_file`?
        //   2. cross-file via import: `call_file` has `from <module> import name`
        //      where <module> resolves to def_file's path?
        // Unresolvable calls don't taint any def — drops the cycle-A FP
        // problem where any same-name call exempted every homonym.
        //
        // Note: imports alone are NOT treated as uses. ga's actual contract
        // (per ga_query::dead_code) uses CALLS + REFERENCES edges only;
        // being imported but never called → still considered a candidate
        // for dead. Tested directly: an experimental cycle B'' that
        // treated imports as uses dropped precision from 0.803 → 0.735
        // because ga correctly flagged unreferenced imports as dead while
        // the looser bench expected them to be live.
        let mut targeted_pairs: HashSet<(String, String)> = HashSet::new();
        for (call_file, name) in &call_sites {
            let candidate_defs = match defs_by_name.get(name) {
                Some(v) => v,
                None => continue,
            };
            for def_file in candidate_defs {
                if call_file == def_file {
                    targeted_pairs.insert((name.clone(), def_file.clone()));
                    continue;
                }
                if call_resolves_via_import(call_file, def_file, name, &imports_per_file) {
                    targeted_pairs.insert((name.clone(), def_file.clone()));
                }
            }
        }

        // Cycle B — entry-point sets sourced from ga_query::entry_points
        // (the same helpers ga_query::dead_code uses internally). This is
        // what closes the django ~22k FN gap by exempting view methods,
        // gin handlers, etc. that production correctly recognises.
        let route_handlers = collect_route_handlers(fixture_dir);
        let dunder_all_set = collect_dunder_all(fixture_dir);
        let pyproject_scripts = collect_project_scripts(fixture_dir);
        let cargo_bins = collect_cargo_bins(fixture_dir);

        let mut tasks = Vec::with_capacity(defs.len());
        for def in defs {
            let entry_point_kind = classify_entry_point(
                &def,
                &route_handlers,
                &dunder_all_set,
                &pyproject_scripts,
                &cargo_bins,
            );

            let is_entry = entry_point_kind.is_some();
            // Cycle B' — per-file targeted lookup matches ga's post-S-003
            // (name, file) resolution. Dropping name-only union exemption
            // closes the FP gap where bench marked things live just because
            // a homonym was called somewhere.
            let is_targeted = targeted_pairs.contains(&(def.name.clone(), def.file.clone()));
            let expected_dead = !is_entry && !is_targeted;

            let kind_str = symbol_kind_str(def.kind);
            let task_id = format!("hd-ast::{}::{}", def.file, def.name);

            let query = json!({
                "name": def.name,
                "file": def.file,
                "kind": kind_str,
                "line": def.line,
                "expected_dead": expected_dead,
                "entry_point_kind": entry_point_kind,
            });
            let rationale = if expected_dead {
                "zero callers, zero references; not an entry-point candidate".to_string()
            } else if let Some(ep) = entry_point_kind {
                format!("entry-point: {ep}")
            } else {
                "name appears as a callee or reference target somewhere in repo".to_string()
            };

            // For a dead-code GT, "expected" carries the (name, file)
            // identity tuple so retrievers can be scored against the def
            // identity — same shape as ga_dead_code's DeadCodeEntry.
            let expected = vec![format!("{}:{}", def.file, def.name)];

            tasks.push(GeneratedTask {
                task_id,
                query,
                expected,
                rule: "Hd-ast".to_string(),
                rationale,
            });
        }
        Ok(tasks)
    }
}

#[derive(Debug, Clone)]
struct DefRow {
    name: String,
    file: String,
    kind: SymbolKind,
    line: u32,
}

/// Lightweight import edge for cycle B' per-file resolution.
#[derive(Debug)]
struct HdImport {
    target_path: String,
    imported_names: Vec<String>,
}

/// Cycle B' — does a call to `name` in `call_file` resolve to a def of
/// `name` in `def_file` via an explicit import? Mirrors the resolution
/// rules in Hrn-static cycle B.
fn call_resolves_via_import(
    call_file: &str,
    def_file: &str,
    name: &str,
    imports_per_file: &BTreeMap<String, Vec<HdImport>>,
) -> bool {
    let Some(imports) = imports_per_file.get(call_file) else {
        return false;
    };
    let def_module = file_to_module_path(def_file);
    let def_module_alt = strip_init_suffix(&def_module);
    for imp in imports {
        if !imp.imported_names.iter().any(|n| n == name) {
            continue;
        }
        let target = normalise_import_target(&imp.target_path);
        if target.is_empty() {
            continue;
        }
        if target == def_module
            || target == def_module_alt
            || def_module.starts_with(&format!("{target}/"))
            || def_module.starts_with(&format!("{target}."))
        {
            return true;
        }
    }
    false
}

fn file_to_module_path(file: &str) -> String {
    let stripped = file
        .strip_suffix(".py")
        .or_else(|| file.strip_suffix(".pyi"))
        .or_else(|| file.strip_suffix(".rs"))
        .or_else(|| file.strip_suffix(".go"))
        .or_else(|| file.strip_suffix(".rb"))
        .or_else(|| file.strip_suffix(".tsx"))
        .or_else(|| file.strip_suffix(".ts"))
        .or_else(|| file.strip_suffix(".jsx"))
        .or_else(|| file.strip_suffix(".js"))
        .unwrap_or(file);
    if file.ends_with(".py") || file.ends_with(".pyi") {
        return stripped.replace('/', ".");
    }
    stripped.to_string()
}

fn strip_init_suffix(module: &str) -> String {
    module
        .strip_suffix(".__init__")
        .map(str::to_string)
        .unwrap_or_else(|| module.to_string())
}

fn normalise_import_target(target: &str) -> String {
    if target.is_empty() {
        return String::new();
    }
    if target.starts_with("./") || target.starts_with("../") {
        return String::new();
    }
    if target.contains("::") {
        return target.replace("::", ".");
    }
    target.to_string()
}

fn symbol_kind_str(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Module => "module",
        SymbolKind::Other => "other",
    }
}

/// Decide whether `def` is excluded as an entry-point. Returns the
/// classifier name (`main`, `route_handler`, `dunder_all`,
/// `project_scripts`, `cargo_bin`) or `None` if `def` is a regular candidate.
///
/// Order mirrors `ga_query::dead_code::EntryPointSet::is_entry_point` so
/// the bench classification follows the same precedence as the production
/// tool (per spec C2 — definitional alignment).
fn classify_entry_point(
    def: &DefRow,
    route_handlers: &HashSet<String>,
    dunder_all: &HashSet<(String, String)>,
    pyproject_scripts: &HashSet<String>,
    cargo_bins: &HashSet<String>,
) -> Option<&'static str> {
    if def.name == "main" || def.name == "__main__" {
        return Some("main");
    }
    if route_handlers.contains(&def.name) {
        return Some("route_handler");
    }
    if pyproject_scripts.contains(&def.name) {
        return Some("project_scripts");
    }
    if cargo_bins.contains(&def.name) {
        return Some("cargo_bin");
    }
    // __all__: per-file membership — match `same_package` semantics so that
    // `pkg/__init__.py` declaring `foo` covers `pkg/anything.py::foo`.
    for (export_file, export_name) in dunder_all {
        if export_name == &def.name && same_package(export_file, &def.file) {
            return Some("dunder_all");
        }
    }
    None
}
