//! Hrn-static — `ga_rename_safety` GT rule for M3 bench (S-006).
//!
//! ## Anti-tautology policy
//! This rule does NOT import `ga_query::{dead_code, callers, rename_safety,
//! architecture, risk, minimal_context}` analysis types. Sites + blockers
//! are computed from raw AST + line-local source scan:
//! - sites:    `ga_parser::extract_calls` + `ga_parser::extract_references`
//! - blockers: line-local string-literal scan (semantics matches
//!   `ga_query::rename_safety::file_has_string_literal` per AS-018; the
//!   helper is duplicated here, not imported, to keep the rule independent).
//!
//! See spec §C1 + AS-017/AS-018.
//!
//! ## Identity (post-S-003)
//! Each GT entry is keyed by `(target, def_file)`. Polymorphic targets
//! (≥2 def files for the same name) are tiered separately so retrievers
//! that handle polymorphism well can be scored against the harder cases.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::calls::extract_calls;
use ga_parser::imports::extract_imports;
use ga_parser::references::extract_references;
use ga_parser::walk::walk_repo;
use ga_parser::{parse_source, ParsedSymbol};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct HrnStatic;

impl Default for HrnStatic {
    fn default() -> Self {
        Self
    }
}

impl GtRule for HrnStatic {
    fn id(&self) -> &str {
        "Hrn-static"
    }
    fn uc(&self) -> &str {
        "rename_safety"
    }
    fn policy_bias(&self) -> &str {
        "Hrn-static cycle B — sites from raw AST (extract_calls + extract_references); \
         per-def filtering: a call site `foo()` in file F resolves to def \
         (foo, def_file) iff (a) F == def_file (intra-file), or (b) F has an \
         import linking back to def_file's package (`from <pkg>.<def_module> import foo` \
         or aliased equivalents). Cross-file calls without resolvable imports \
         are dropped from `expected_sites` for ALL homonyms — under-counts \
         expected sites rather than over-attributing to wrong def. Blockers \
         from line-local string-literal scan: multi-line string blockers \
         (triple-quoted Python, backtick-template TS) are NOT detected — \
         false-negatives in GT documented honestly per C4. Polymorphic tier \
         (≥2 def files for same name) scored separately."
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // Pass 1 — collect defs + per-file source + imports.
        let mut defs_by_name: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        let mut source_per_file: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut file_lang: BTreeMap<String, ga_core::Lang> = BTreeMap::new();
        let mut imports_per_file: BTreeMap<String, Vec<ImportLink>> = BTreeMap::new();

        for entry in &report.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            let bytes = match std::fs::read(&entry.abs_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            file_lang.insert(rel.clone(), entry.lang);

            if let Ok(symbols) = parse_source(entry.lang, &bytes) {
                for sym in symbols {
                    let ParsedSymbol {
                        name, kind, line, ..
                    } = sym;
                    if name.is_empty() || !is_renameable_kind(kind) {
                        continue;
                    }
                    defs_by_name
                        .entry(name)
                        .or_default()
                        .push((rel.clone(), line));
                }
            }
            if let Ok(imps) = extract_imports(entry.lang, &bytes) {
                let links: Vec<ImportLink> = imps
                    .into_iter()
                    .filter_map(|i| {
                        if i.imported_names.is_empty() {
                            return None;
                        }
                        Some(ImportLink {
                            target_path: i.target_path,
                            imported_names: i.imported_names,
                        })
                    })
                    .collect();
                if !links.is_empty() {
                    imports_per_file.insert(rel.clone(), links);
                }
            }
            source_per_file.insert(rel, bytes);
        }

        // Pass 2 — collect sites by (callee_name, call_file, line).
        let mut sites_by_name: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        for (rel, bytes) in &source_per_file {
            let lang = match file_lang.get(rel) {
                Some(l) => *l,
                None => continue,
            };
            if let Ok(calls) = extract_calls(lang, bytes) {
                for c in calls {
                    if !c.callee_name.is_empty() {
                        sites_by_name
                            .entry(c.callee_name)
                            .or_default()
                            .push((rel.clone(), c.call_site_line));
                    }
                }
            }
            if let Ok(refs) = extract_references(lang, bytes) {
                for r in refs {
                    if !r.target_name.is_empty() {
                        sites_by_name
                            .entry(r.target_name)
                            .or_default()
                            .push((rel.clone(), r.ref_site_line));
                    }
                }
            }
        }

        // Pass 3 — emit one GT task per (name, def_file). Sites are filtered
        // PER DEF: intra-file calls + cross-file calls whose import resolves
        // to def_file's module.
        let mut out: Vec<GeneratedTask> = Vec::new();
        for (name, def_locations) in &defs_by_name {
            let unique_files: BTreeSet<&str> =
                def_locations.iter().map(|(f, _)| f.as_str()).collect();
            let def_kind = if unique_files.len() <= 1 {
                "unique"
            } else {
                "polymorphic"
            };

            let raw_sites = sites_by_name.get(name).cloned().unwrap_or_default();

            // Blockers — per file, line-local string-literal scan. Skip the
            // def file's def line (def-site contains the name in code form,
            // not a string literal).
            let def_line_per_def: BTreeMap<&str, u32> = def_locations
                .iter()
                .map(|(f, l)| (f.as_str(), *l))
                .collect();
            let mut blockers: Vec<Value> = Vec::new();
            for (rel, bytes) in &source_per_file {
                let text = match std::str::from_utf8(bytes) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if file_has_string_literal_skip_line(
                    text,
                    name,
                    def_line_per_def.get(rel.as_str()).copied(),
                ) {
                    blockers.push(json!({"file": rel, "reason": "string_literal"}));
                }
            }
            blockers.sort_by(|a, b| {
                a.get("file")
                    .and_then(|v| v.as_str())
                    .cmp(&b.get("file").and_then(|v| v.as_str()))
            });

            for (def_file, def_line) in def_locations {
                // Per-def filter: keep only sites that resolve to THIS def.
                let per_def_sites: Vec<(String, u32)> = raw_sites
                    .iter()
                    .filter(|(call_file, _)| {
                        site_resolves_to_def(call_file, def_file, name, &imports_per_file)
                    })
                    .cloned()
                    .collect();
                let mut sites: Vec<Value> = per_def_sites
                    .iter()
                    .map(|(f, l)| json!({"file": f, "line": l}))
                    .collect();
                sites.sort_by(|a, b| {
                    a.get("file")
                        .and_then(|v| v.as_str())
                        .cmp(&b.get("file").and_then(|v| v.as_str()))
                        .then(
                            a.get("line")
                                .and_then(|v| v.as_u64())
                                .cmp(&b.get("line").and_then(|v| v.as_u64())),
                        )
                });
                let task_id = format!("hrn-static::{}::{}", def_file, name);
                let query = json!({
                    "target": name,
                    "file": def_file,
                    "line": def_line,
                    "def_kind": def_kind,
                    "expected_sites": sites,
                    "expected_blockers": blockers,
                });
                let rationale = format!(
                    "{def_kind} target `{name}` defined at {def_file}:{def_line}; \
                     sites filtered to those resolving to this def via intra-file or import"
                );
                let expected: Vec<String> = per_def_sites
                    .iter()
                    .map(|(f, l)| format!("{f}:{l}"))
                    .collect();
                out.push(GeneratedTask {
                    task_id,
                    query,
                    expected,
                    rule: "Hrn-static".to_string(),
                    rationale,
                });
            }
        }
        Ok(out)
    }
}

/// Lightweight import edge for site-resolution: just `target_path` +
/// `imported_names`. Aliases ignored — bench can't track local rename.
#[derive(Debug)]
struct ImportLink {
    target_path: String,
    imported_names: Vec<String>,
}

/// Per-def site filter — used in pass 3 to prune sites that don't
/// resolve to (name, def_file).
///
/// Resolution rules (best-effort, no graph queries):
/// 1. Intra-file: `call_file == def_file` → resolves.
/// 2. Cross-file with explicit import: `call_file` has `from <module> import name`
///    where `<module>` resolves to `def_file`'s file path → resolves.
/// 3. Otherwise → does NOT resolve (drop).
///
/// Conservative: under-counts cross-file sites without parsable imports
/// (e.g. dynamic imports, `import *` re-exports, attribute access). The
/// trade-off is favoured over attribution to wrong def per spec C2 +
/// AS-018 (sites must match shipped tool semantically).
fn site_resolves_to_def(
    call_file: &str,
    def_file: &str,
    name: &str,
    imports_per_file: &BTreeMap<String, Vec<ImportLink>>,
) -> bool {
    if call_file == def_file {
        return true;
    }
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
        if target == def_module || target == def_module_alt || target.is_empty() {
            // Empty target = relative import; treat as unresolvable but
            // not a hard mismatch — allow to be conservative-ish on TS.
            // Switching to `==` only would drop too many real edges in
            // mixed-langs fixtures. Use prefix match for safety.
            if target == def_module
                || target == def_module_alt
                || (!target.is_empty()
                    && (def_module.starts_with(&format!("{target}/"))
                        || def_module.starts_with(&format!("{target}."))))
            {
                return true;
            }
        }
    }
    false
}

/// File path → import-style module path.
/// `auth/models.py` → `auth.models`
/// `auth/__init__.py` → `auth`
/// `pkg/foo.ts` → `pkg/foo`
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
    // Python convention: dot-separated.
    if file.ends_with(".py") || file.ends_with(".pyi") {
        return stripped.replace('/', ".");
    }
    stripped.to_string()
}

/// `auth.__init__` → `auth`
fn strip_init_suffix(module: &str) -> String {
    module
        .strip_suffix(".__init__")
        .map(str::to_string)
        .unwrap_or_else(|| module.to_string())
}

/// Normalise an import target to match `file_to_module_path` shape.
fn normalise_import_target(target: &str) -> String {
    if target.is_empty() {
        return String::new();
    }
    if target.starts_with("./") || target.starts_with("../") {
        return String::new(); // relative; unresolvable by simple match
    }
    if target.contains("::") {
        return target.replace("::", ".");
    }
    target.to_string()
}

fn is_renameable_kind(kind: ga_core::SymbolKind) -> bool {
    use ga_core::SymbolKind::*;
    matches!(
        kind,
        Function | Method | Class | Struct | Trait | Interface | Enum
    )
}

/// Mirrors `ga_query::rename_safety::file_has_string_literal` semantics —
/// scans single-line literals (single + double quote) for word-bounded
/// occurrences of `target`. Skips `skip_line` (1-based) so the def-site
/// itself isn't flagged. Multi-line literals are NOT detected — see
/// policy_bias.
fn file_has_string_literal_skip_line(text: &str, target: &str, skip_line: Option<u32>) -> bool {
    for (idx, line) in text.lines().enumerate() {
        let line_no = (idx as u32) + 1;
        if Some(line_no) == skip_line {
            continue;
        }
        if !line.contains('"') && !line.contains('\'') {
            continue;
        }
        if line_contains_string_literal_for(line, target) {
            return true;
        }
    }
    false
}

fn line_contains_string_literal_for(line: &str, target: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            let quote = c;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                j += 1;
            }
            if j > start {
                let body = &line[start..j.min(line.len())];
                if literal_contains_word(body, target) {
                    return true;
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    false
}

fn literal_contains_word(text: &str, target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = text[start..].find(target) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !text.as_bytes()[abs - 1].is_ascii_alphanumeric()
                && text.as_bytes()[abs - 1] != b'_';
        let after_idx = abs + target.len();
        let after_ok = after_idx >= text.len()
            || (!text.as_bytes()[after_idx].is_ascii_alphanumeric()
                && text.as_bytes()[after_idx] != b'_');
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}
