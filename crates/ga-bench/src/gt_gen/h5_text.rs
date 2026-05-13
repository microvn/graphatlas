//! H5-text — raw-AST re-export chain importers rule. Unbiased replacement for
//! `h5_reexport.rs` which used `ga_query::importers` (graph-query) → made GA
//! F1=1.000 tautologically on importers UC.
//!
//! ## Policy
//!
//! For each file F in the repo, walk every source file, extract its imports,
//! resolve each import's `target_path` to a repo-local file via the shared
//! [`ga_query::import_resolve`] helper (same logic the indexer uses, but
//! invoked directly from AST parse output — no graph lookup). Build an
//! in-memory adjacency: `file -> [(importer_file, is_re_export)]`.
//!
//! Then for each leaf file F:
//!   - Direct importers = files that import F
//!   - Transitive importers = files that import via a `is_re_export=true` hop
//!     (depth ≤ 3 per Tools-C12)
//! If there's ≥ 1 re_export hop AND total importers ≥ 2 → emit task.
//!   expected = all files reaching F (direct + transitive)
//!
//! Every retriever scored against the same structural adjacency. GA no
//! longer auto-wins; it competes against the AST-derived ground truth.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::imports::extract_imports;
use ga_parser::walk::walk_repo;
use ga_query::import_resolve::{resolve_pending_imports, PendingImport};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct H5Text;

impl GtRule for H5Text {
    fn id(&self) -> &str {
        "H5-text"
    }
    fn uc(&self) -> &str {
        "importers"
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // Collect all pending imports across every file (mirrors what the
        // indexer does in build_index, but we never write to a graph DB).
        let mut pending: Vec<PendingImport> = Vec::new();
        let mut file_paths: HashSet<String> = HashSet::new();
        for entry in &report.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            file_paths.insert(rel.clone());
            let Ok(bytes) = std::fs::read(&entry.abs_path) else {
                continue;
            };
            let Ok(imps) = extract_imports(entry.lang, &bytes) else {
                continue;
            };
            for imp in imps {
                pending.push(PendingImport {
                    src_file: rel.clone(),
                    src_lang: entry.lang,
                    target_path: imp.target_path,
                    import_line: imp.import_line,
                    imported_names: imp.imported_names,
                    imported_aliases: imp.imported_aliases,
                    is_re_export: imp.is_re_export,
                    type_only_names: imp.type_only_names,
                });
            }
        }

        // Resolve to (src, dst, is_re_export) tuples — external/stdlib drop.
        let rows = resolve_pending_imports(&pending, &file_paths);

        // Build reverse adjacency: dst -> [(src, re_export)]
        let mut direct_importers: HashMap<String, Vec<(String, bool)>> = HashMap::new();
        for (src, dst, _line, _names, re_export) in &rows {
            direct_importers
                .entry(dst.clone())
                .or_default()
                .push((src.clone(), *re_export));
        }

        // For each leaf file: compute transitive importers depth ≤ 3 via
        // re_export hops. BFS walking up from target.
        let mut out = Vec::new();
        for leaf in direct_importers.keys() {
            let (all_importers, has_chain) = transitive_importers(leaf, &direct_importers);
            if !has_chain || all_importers.len() < 2 {
                continue; // not a re-export chain case
            }
            let mut expected: Vec<String> = all_importers.into_iter().collect();
            expected.sort();
            out.push(GeneratedTask {
                task_id: format!("importers_{}", sanitize(leaf)),
                query: json!({ "file": leaf }),
                expected,
                rule: self.id().to_string(),
                rationale: format!(
                    "file `{}` reached by ≥2 importers with ≥1 re_export hop (raw-AST resolver)",
                    leaf
                ),
            });
        }
        Ok(out)
    }
}

/// BFS up to depth 3. Returns (set of all importers reaching target, has_chain).
fn transitive_importers(
    target: &str,
    adj: &HashMap<String, Vec<(String, bool)>>,
) -> (HashSet<String>, bool) {
    let mut visited: HashSet<String> = HashSet::new();
    let mut frontier: Vec<(String, u8, bool)> = Vec::new(); // (file, depth, path_has_re_export)
    let mut has_chain = false;
    if let Some(direct) = adj.get(target) {
        for (src, re_export) in direct {
            frontier.push((src.clone(), 1, *re_export));
            if *re_export {
                has_chain = true;
            }
        }
    }
    while let Some((file, depth, path_re_export)) = frontier.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        if depth >= 3 {
            continue;
        }
        if let Some(deeper) = adj.get(&file) {
            for (src, re_export) in deeper {
                if visited.contains(src) {
                    continue;
                }
                let chain = path_re_export || *re_export;
                if chain {
                    has_chain = true;
                }
                frontier.push((src.clone(), depth + 1, chain));
            }
        }
    }
    (visited, has_chain)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
