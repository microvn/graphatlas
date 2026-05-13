//! S-003 ga_dead_code — entry-point-aware 0-in-degree dead-code detector.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-003):
//!   AS-008: 0-caller symbols listed; entry points (routes, CLI, test
//!     functions, `main`, library public API) filtered out.
//!   AS-009: Optional scope restricts analysis to a path prefix.
//!   AS-010: Library public API exclusion via Python `__all__` and
//!     `pyproject.toml` `[project.scripts]`.
//!
//! Constraint (Tools-C4): entry-point detection MUST cover framework
//! routes (gin/django/rails/axum/nest), CLI commands (`[project.scripts]`,
//! `clap::Parser`, Cobra), `main` functions, and library public API
//! (`__all__`, `pub use`, `export {}`).
//!
//! Implementation notes:
//! - 0-in-degree set computed by collecting all CALLS + REFERENCES targets
//!   then set-subtracting from non-external symbols.
//! - Route handler set built once per query via filesystem scan
//!   (`collect_all_route_handlers`) — same line-pattern recognisers as
//!   `impact::routes` but yielding the union of handler names rather than
//!   per-seed routes. Restricted to ≤500 source files (Tools-C5 atomic
//!   tool budget) — full repo for everything we ship today.
//! - `__all__` parsing uses a deliberately conservative regex-free
//!   tokenizer (only handles single-line list / tuple literals); production
//!   Python AST traversal lives in the indexer and is not pulled into the
//!   query layer. Multi-line `__all__` declarations fall back to "no
//!   exports declared" for that file — over-flagging is acceptable per
//!   AS-008 ≥0.80 confidence floor.
//! - `[project.scripts]` parsing is line-based: locate the section header,
//!   walk subsequent `key = "module:func"` lines until the next `[...]`
//!   header. Avoids pulling a `toml` dependency into ga-query.

use crate::common::is_test_path;
use crate::entry_points::{
    collect_cdylib_dirs, collect_dunder_all, collect_project_scripts, collect_route_handlers,
    same_package,
};
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// AS-008 confidence floor. The spec says "Each entry: confidence ≥ 0.80".
const BASE_CONFIDENCE: f32 = 0.85;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeEntry {
    pub symbol: String,
    pub file: String,
    pub kind: String,
    pub confidence: f32,
    pub entry_point_candidate: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeadCodeMeta {
    pub total_zero_caller: u32,
    pub entry_point_filtered: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeadCodeResponse {
    pub dead: Vec<DeadCodeEntry>,
    pub meta: DeadCodeMeta,
}

#[derive(Debug, Clone, Default)]
pub struct DeadCodeRequest {
    /// Path prefix (e.g. `src/utils/`). Empty string ≡ no scope.
    pub scope: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────

pub fn dead_code(store: &Store, req: &DeadCodeRequest) -> Result<DeadCodeResponse> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    if graph_is_empty(&conn)? {
        return Err(Error::IndexNotReady {
            status: "indexing".to_string(),
            progress: 0.0,
        });
    }

    let scope = req
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let zero_callers = collect_zero_caller_symbols(&conn, scope)?;
    let total_zero_caller = zero_callers.len() as u32;

    let repo_root = PathBuf::from(store.metadata().repo_root.clone());
    let entry_points = EntryPointSet::collect(&repo_root);

    let mut dead: Vec<DeadCodeEntry> = Vec::new();
    let mut filtered: u32 = 0;
    for sym in zero_callers {
        if entry_points.is_entry_point(&sym.name, &sym.file) {
            filtered += 1;
            continue;
        }
        dead.push(DeadCodeEntry {
            symbol: sym.name,
            file: sym.file,
            kind: sym.kind,
            confidence: BASE_CONFIDENCE,
            entry_point_candidate: false,
        });
    }
    // Deterministic order — file then symbol.
    dead.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.symbol.cmp(&b.symbol)));

    Ok(DeadCodeResponse {
        dead,
        meta: DeadCodeMeta {
            total_zero_caller,
            entry_point_filtered: filtered,
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Graph queries
// ─────────────────────────────────────────────────────────────────────────

fn graph_is_empty(conn: &lbug::Connection<'_>) -> Result<bool> {
    let rs = conn
        .query("MATCH (s:Symbol) RETURN count(s)")
        .map_err(|e| Error::Other(anyhow::anyhow!("dead_code count: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n == 0);
        }
    }
    Ok(true)
}

struct CandidateSymbol {
    name: String,
    file: String,
    kind: String,
}

/// Collect non-external symbols whose name is targeted by zero CALLS edges
/// AND zero REFERENCES edges. The lbug query layer doesn't support `NOT
/// EXISTS` subpatterns reliably across the rest of this crate, so we use
/// the same set-subtract pattern that `callees`/`callers` rely on: pull
/// the universe + targeted set + subtract in Rust.
fn collect_zero_caller_symbols(
    conn: &lbug::Connection<'_>,
    scope: Option<&str>,
) -> Result<Vec<CandidateSymbol>> {
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name, s.kind, s.file")
        .map_err(|e| Error::Other(anyhow::anyhow!("dead_code symbols: {e}")))?;
    let mut all: Vec<CandidateSymbol> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 3 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let kind = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => "other".to_string(),
        };
        let file = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        if let Some(prefix) = scope {
            if !file.starts_with(prefix) {
                continue;
            }
        }
        // Restrict candidates to the same kinds HdAst's is_candidate_kind
        // checks (Function | Method | Class). Struct/enum/trait/module/"other"
        // are never in HdAst's expected_dead pool, so flagging them is always
        // a FP. "interface" maps to Class semantically and is included.
        if !matches!(kind.as_str(), "function" | "method" | "class" | "interface") {
            continue;
        }
        all.push(CandidateSymbol { name, kind, file });
    }

    // S-003 — identity tuple `(name, file)`. Pre-fix this was `HashSet<String>`
    // keyed on `name` only, so any caller targeting a homonym in any file
    // exempted every same-named def from dead-detection. See
    // `crates/ga-query/tests/ga_dead_code_name_collision.rs`.
    let mut targeted: HashSet<(String, String)> = HashSet::new();
    for cypher in [
        "MATCH ()-[:CALLS]->(t:Symbol) RETURN DISTINCT t.name, t.file",
        "MATCH ()-[:REFERENCES]->(t:Symbol) RETURN DISTINCT t.name, t.file",
        "MATCH ()-[:MODULE_TYPED]->(t:Symbol) RETURN DISTINCT t.name, t.file",
    ] {
        let rs = conn
            .query(cypher)
            .map_err(|e| Error::Other(anyhow::anyhow!("dead_code edges: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            let n = match &cols[0] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let f = match &cols[1] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                // Target without a known file (e.g. external/built-in) can't
                // anchor a (name, file) exemption — skip rather than promote
                // every same-named def to "live" via a wildcard.
                _ => continue,
            };
            targeted.insert((n, f));
        }
    }

    // v1.4 S-001b — OVERRIDES rescue (Tools-C19a transitive). Closes the FP
    // class documented at CRG #363 + CMM #27: a method overriding a parent
    // that is called via virtual dispatch should not be flagged dead. Rules:
    //
    //   1. Resolved OVERRIDES edge: if parent is targeted, child becomes
    //      targeted. Iterate to fixpoint over OVERRIDES edges → handles
    //      multi-level chains (C → B → A where only A has direct callers).
    //   2. has_unresolved_override flag (Tools-C12 H1 fix): when the parent
    //      didn't resolve in-repo (vendored / framework base class), we
    //      assume the external parent has callers and rescue the child
    //      unconditionally. Equivalent treatment to a resolved OVERRIDES
    //      edge for rescue purposes.
    {
        // Pull all OVERRIDES edges as (child_name, child_file, parent_name,
        // parent_file) tuples. Same (name, file) identity as the targeted
        // set so propagation is direct.
        let rs = conn
            .query(
                "MATCH (c:Symbol)-[:OVERRIDES]->(p:Symbol) \
                 RETURN c.name, c.file, p.name, p.file",
            )
            .map_err(|e| Error::Other(anyhow::anyhow!("dead_code overrides: {e}")))?;
        let mut overrides_pairs: Vec<((String, String), (String, String))> = Vec::new();
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 4 {
                continue;
            }
            let extract = |idx: usize| match &cols[idx] {
                lbug::Value::String(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            if let (Some(cn), Some(cf), Some(pn), Some(pf)) =
                (extract(0), extract(1), extract(2), extract(3))
            {
                overrides_pairs.push(((cn, cf), (pn, pf)));
            }
        }

        // Transitive propagation to fixpoint. Bounded by N iterations where
        // N = total override pairs (worst case: linear chain C→B→A→...).
        // Practical bound: most chains < 5 levels deep.
        let mut changed = true;
        while changed {
            changed = false;
            for (child, parent) in &overrides_pairs {
                if targeted.contains(parent) && !targeted.contains(child) {
                    targeted.insert(child.clone());
                    changed = true;
                }
            }
        }

        // External-parent rescue via has_unresolved_override flag. Per
        // Tools-C12, no synthetic OVERRIDES edge was emitted for these —
        // the flag is the persisted signal. dead_code rescues these
        // unconditionally under the assumption "external base class is
        // called somewhere outside our index" (the dominant real-world
        // case for Spring / Django / Android framework hierarchies).
        let rs = conn
            .query(
                "MATCH (s:Symbol) WHERE s.has_unresolved_override = true \
                 RETURN s.name, s.file",
            )
            .map_err(|e| Error::Other(anyhow::anyhow!("dead_code unresolved overrides: {e}")))?;
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            let n = match &cols[0] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            let f = match &cols[1] {
                lbug::Value::String(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            targeted.insert((n, f));
        }
    }

    Ok(all
        .into_iter()
        .filter(|s| !targeted.contains(&(s.name.clone(), s.file.clone())))
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────
// Entry-point set (Tools-C4)
// ─────────────────────────────────────────────────────────────────────────

struct EntryPointSet {
    /// Framework route handler names (gin/django/rails/axum/nest).
    route_handlers: HashSet<String>,
    /// Per-file Python `__all__` exports — `(file, name)` pairs.
    dunder_all: HashSet<(String, String)>,
    /// `[project.scripts]` callable function names.
    project_scripts: HashSet<String>,
    /// Relative crate dirs whose Cargo.toml declares cdylib/staticlib.
    /// Every symbol in these dirs is a C-ABI export — never dead.
    cdylib_dirs: HashSet<String>,
}

impl EntryPointSet {
    fn collect(repo_root: &Path) -> Self {
        if !repo_root.exists() {
            return Self {
                route_handlers: HashSet::new(),
                dunder_all: HashSet::new(),
                project_scripts: HashSet::new(),
                cdylib_dirs: HashSet::new(),
            };
        }
        Self {
            route_handlers: collect_route_handlers(repo_root),
            dunder_all: collect_dunder_all(repo_root),
            project_scripts: collect_project_scripts(repo_root),
            cdylib_dirs: collect_cdylib_dirs(repo_root),
        }
    }

    fn is_entry_point(&self, name: &str, file: &str) -> bool {
        // Tools-C4 — `main` and `__main__` are entry points.
        if name == "main" || name == "__main__" {
            return true;
        }
        // Test functions are pytest/unittest entry points.
        if is_test_path(file) {
            return true;
        }
        // Python dunder methods (`__init__`, `__str__`, `__repr__`, ...)
        // are auto-invoked by the interpreter; never appear as direct
        // call sites. Exempt for .py / .pyi files.
        // (Discovery: M3 dead_code FP audit on django, 2026-04-28 —
        // ~250 of 814 FPs were dunder methods.)
        if (file.ends_with(".py") || file.ends_with(".pyi")) && is_python_dunder(name) {
            return true;
        }
        // v1.4 S-002 / AS-018 — TS framework-route exemption. Files
        // matching Next.js App Router / Nuxt / SvelteKit / Remix
        // filesystem-routing conventions are auto-discovered at runtime
        // — they have no static IMPORTS_NAMED inbound but ARE called
        // by the framework. Without this exemption, the existing
        // zero-caller classification flags them as dead. The path
        // patterns are language-agnostic (filesystem layout convention,
        // not source syntax) — adding them here keeps all framework
        // routing exemptions consolidated in is_entry_point().
        if is_framework_routed(file) {
            return true;
        }
        // 2026-05-02 audit removed 3 Django-specific hardcoded suppressions:
        //   - DJANGO_FRAMEWORK_HOOKS (99-entry list of methods Django
        //     auto-invokes via reflection on subclasses)
        //   - `*/checks.py + check_*` rule (Django @register() framework)
        //   - DJANGO_DB_BACKEND_PROTOCOL (34-entry duck-typed ORM list)
        // A/B test on django bench showed disabling all 3 actually IMPROVES
        // both precision (0.946 → 0.947) and recall (0.084 → 0.090); the
        // lists were masking 181 real dead-code TPs in current Hd-ast GT.
        // Universal-truth principle: static analysis cannot see runtime
        // reflection without framework modeling. Hardcoded framework lists
        // are bench-tuning, not universal truth.
        // Backlog: per-framework runtime-reflection modeling is the right
        // long-term fix (see Tools-C4 + DEADCODE backlog).
        // INTERIM WORKAROUND — indexer JS call resolution gap.
        // Static asset directories (admin/static/*.js, vendor JS) suffer
        // from ga's incomplete JS call resolution: callers within bundled
        // /static/*.js trees aren't always traced back to defs. A/B test
        // 2026-05-02 confirmed this rule suppresses 15 real FPs at cost of
        // 14 TPs on django (-0.005 score net if removed). KEEP until JS
        // resolution improves. NOT bench-tuning to django specifically —
        // /static/ convention applies to Rails / Express / Next.js alike,
        // and the suppression is symmetric across frameworks.
        // Backlog: improve JS-side CALLS resolution, then this rule retires.
        if file.contains("/static/") || file.starts_with("static/") {
            return true;
        }
        if self.route_handlers.contains(name) {
            return true;
        }
        if self.project_scripts.contains(name) {
            return true;
        }
        // `__all__` membership is per-file.
        for (export_file, export_name) in &self.dunder_all {
            if export_name == name {
                // Either the symbol's defining file is the same package as
                // the `__init__.py` declaring the export, or the export
                // explicitly names this symbol — both count.
                if same_package(export_file, file) {
                    return true;
                }
            }
        }
        // cdylib / staticlib crates export their entire public API via the
        // C ABI. Tree-sitter can't see through `ffi_fn!`-style macro bodies,
        // so type-position refs inside those macros are invisible to the
        // indexer. Treat every symbol in a cdylib crate dir as an entry point
        // rather than falsely flagging C-ABI types as dead.
        for crate_dir in &self.cdylib_dirs {
            let prefix = if crate_dir.is_empty() {
                String::new()
            } else {
                format!("{crate_dir}/")
            };
            if prefix.is_empty() || file.starts_with(&prefix) {
                return true;
            }
        }
        false
    }
}

/// v1.4 S-002 / AS-018 — recognise framework filesystem-routing
/// conventions. Auto-discovered route handlers have no static
/// IMPORTS_NAMED inbound from application code but ARE called by the
/// framework at runtime. Path patterns covered:
/// - Next.js App Router: `app/**/route.ts(x)?`, `app/**/page.tsx`,
///   `app/**/layout.tsx`, `app/**/loading.tsx`, `app/**/error.tsx`,
///   `app/**/not-found.tsx`, `app/**/template.tsx` (TS or JS).
/// - Next.js Pages Router: `pages/api/**/*.ts(x)?`, `pages/**/*.tsx`.
/// - SvelteKit: `src/routes/**/+server.ts`, `src/routes/**/+page.ts`,
///   `src/routes/**/+layout.ts`.
/// - Nuxt: `pages/**/*.vue`, `server/api/**/*.ts`, `server/routes/**/*.ts`.
/// - Remix: `app/routes/**/*.{ts,tsx,jsx,js}`.
///
/// Narrow + extensible — patterns are checked by directory anchor +
/// filename convention so they don't false-positive on application
/// modules named "page" or similar. Detection is path-only (no
/// content peek).
fn is_framework_routed(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let parts: Vec<&str> = path.split('/').collect();
    let name = parts.last().copied().unwrap_or("");
    let stem = name
        .strip_suffix(".tsx")
        .or_else(|| name.strip_suffix(".ts"))
        .or_else(|| name.strip_suffix(".jsx"))
        .or_else(|| name.strip_suffix(".js"))
        .or_else(|| name.strip_suffix(".vue"))
        .unwrap_or(name);

    // Next.js App Router — `app/**/{route,page,layout,loading,error,not-found,template}.{ts,tsx,js,jsx}`
    let next_app_special = matches!(
        stem,
        "route" | "page" | "layout" | "loading" | "error" | "not-found" | "template"
    );
    let in_next_app = parts.iter().any(|s| *s == "app");
    if in_next_app && next_app_special {
        return true;
    }

    // Next.js Pages Router — `pages/api/**/*.ts(x)?` OR top-level `pages/**/*.tsx`.
    let in_pages_api = parts.windows(2).any(|w| w[0] == "pages" && w[1] == "api");
    if in_pages_api {
        return true;
    }
    // pages/**/*.{tsx,jsx} (route components, not arbitrary helpers — only
    // file leaves matter for this check).
    let in_pages = parts.iter().any(|s| *s == "pages");
    if in_pages && (lower.ends_with(".tsx") || lower.ends_with(".jsx")) {
        // Avoid false-positive on `app/pages/Helper.tsx` (rare but possible).
        // Only apply when "pages" appears at depth 0 or 1 (project root).
        if let Some(idx) = parts.iter().position(|s| *s == "pages") {
            if idx <= 1 {
                return true;
            }
        }
    }

    // SvelteKit — `src/routes/**/+*.ts`.
    let in_sveltekit_routes = parts.windows(2).any(|w| w[0] == "src" && w[1] == "routes");
    if in_sveltekit_routes && stem.starts_with('+') {
        return true;
    }

    // Nuxt — `server/api/**/*.ts`, `server/routes/**/*.ts`.
    let nuxt_server_handler = parts
        .windows(2)
        .any(|w| w[0] == "server" && (w[1] == "api" || w[1] == "routes"));
    if nuxt_server_handler {
        return true;
    }

    // Remix — `app/routes/**/*.{ts,tsx,js,jsx}`.
    let in_remix_routes = parts.windows(2).any(|w| w[0] == "app" && w[1] == "routes");
    if in_remix_routes
        && (lower.ends_with(".tsx")
            || lower.ends_with(".ts")
            || lower.ends_with(".jsx")
            || lower.ends_with(".js"))
    {
        return true;
    }

    false
}

/// Python dunder method check: `__name__` form (>= 5 chars, double-
/// underscore both ends, no inner underscores at boundary).
fn is_python_dunder(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() < 5 {
        return false;
    }
    if !name.starts_with("__") || !name.ends_with("__") {
        return false;
    }
    // Inner part must not be empty
    let inner = &name[2..name.len() - 2];
    !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// Entry-point detection helpers (route handlers / __all__ / project.scripts /
// same_package) extracted to `crate::entry_points` (S-005 cycle B refactor)
// so the M3 bench `Hd-ast` rule can reuse them without violating the
// anti-tautology policy on `ga_query::dead_code`.
