//! Tools S-003 cluster A — resolve a raw import string (as written in
//! source) to a repo-local File path, or None for external/unresolved
//! targets. Per-lang best-effort — the rule is: if the resolved candidate
//! matches a File in the graph, use it; else drop.

use ga_core::Lang;
use std::collections::HashSet;

/// Row written into `imports.csv` and the IMPORTS rel table.
/// Tuple: (src_file, dst_file, import_line, imported_names_joined, re_export).
pub type ImportRow = (String, String, i64, String, bool);

pub struct PendingImport {
    pub src_file: String,
    pub src_lang: Lang,
    pub target_path: String,
    pub import_line: u32,
    pub imported_names: Vec<String>,
    /// infra:S-002 AS-005 — `(local, original)` alias pairs from the
    /// parser. Empty when no aliases. Indexer uses these to map a call
    /// site's local name to the original symbol name in the target file.
    pub imported_aliases: Vec<(String, String)>,
    pub is_re_export: bool,
    /// v1.4 S-002 / AS-015..017 — TS-only: subset of `imported_names`
    /// (LOCAL names, post-alias) that carry the `type` modifier. Indexer
    /// cross-references this list to populate `IMPORTS_NAMED.is_type_only`
    /// per-row. Empty for non-TS langs.
    pub type_only_names: Vec<String>,
}

/// Resolve pending imports into CSV-ready edge rows. Skips self-imports and
/// duplicate (src, dst) pairs. Drops anything whose path doesn't resolve to
/// a File in the indexed set (external / stdlib). Tools-C12 scope.
pub fn resolve_pending_imports(
    pending: &[PendingImport],
    file_paths: &HashSet<String>,
) -> Vec<ImportRow> {
    let mut rows = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for pi in pending {
        let Some(dst) = resolve_import_path(&pi.target_path, pi.src_lang, &pi.src_file, file_paths)
        else {
            continue;
        };
        if dst == pi.src_file {
            continue;
        }
        if !seen.insert((pi.src_file.clone(), dst.clone())) {
            continue;
        }
        // lbug CSV: `,` would split columns; join imported_names with `|`.
        let names_joined = pi.imported_names.join("|");
        rows.push((
            pi.src_file.clone(),
            dst,
            pi.import_line as i64,
            names_joined,
            pi.is_re_export,
        ));
    }
    rows
}

pub fn resolve_import_path(
    raw: &str,
    lang: Lang,
    src_file: &str,
    file_paths: &HashSet<String>,
) -> Option<String> {
    match lang {
        Lang::Python => resolve_python_import(raw, file_paths),
        Lang::TypeScript | Lang::JavaScript => resolve_ts_js_import(raw, src_file, file_paths),
        _ => None, // Go / Rust cluster A — deferred
    }
}

fn resolve_python_import(raw: &str, file_paths: &HashSet<String>) -> Option<String> {
    let base = raw.replace('.', "/");
    let candidate_file = format!("{base}.py");
    if file_paths.contains(&candidate_file) {
        return Some(candidate_file);
    }
    let candidate_pkg = format!("{base}/__init__.py");
    if file_paths.contains(&candidate_pkg) {
        return Some(candidate_pkg);
    }
    None
}

fn resolve_ts_js_import(raw: &str, src_file: &str, file_paths: &HashSet<String>) -> Option<String> {
    if !raw.starts_with("./") && !raw.starts_with("../") {
        return None;
    }
    let src_dir = std::path::Path::new(src_file).parent()?;
    let joined = src_dir.join(raw).to_string_lossy().into_owned();
    let cleaned = clean_relative(&joined);
    for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
        let candidate = format!("{cleaned}{ext}");
        if file_paths.contains(&candidate) {
            return Some(candidate);
        }
    }
    for ext in &["index.ts", "index.tsx", "index.js", "index.jsx"] {
        let candidate = format!("{cleaned}/{ext}");
        if file_paths.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn clean_relative(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}
