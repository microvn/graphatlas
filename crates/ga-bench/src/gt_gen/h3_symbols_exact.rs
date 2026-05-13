//! H3-exact — raw-AST symbols MRR rule.
//!
//! For each unique defined symbol name (from `ga_parser::parse_source`),
//! emit `ga_symbols {pattern: name, match: "exact"}` task; expected = [name]
//! so MRR scoring measures how well the tool ranks the target.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::{parse_source, walk::walk_repo};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

const MAX_DEFINING_FILES: usize = 5;
const MAX_TASKS_PER_FIXTURE: usize = 200;

#[derive(Default)]
pub struct H3SymbolsExact;

impl GtRule for H3SymbolsExact {
    fn id(&self) -> &str {
        "H3-symbols-exact"
    }
    fn uc(&self) -> &str {
        "symbols"
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let walk = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // name → set<file> — drop names defined in too many files (too generic).
        let mut name_files: HashMap<String, BTreeSet<String>> = HashMap::new();
        for entry in &walk.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            let Ok(bytes) = std::fs::read(&entry.abs_path) else {
                continue;
            };
            let Ok(symbols) = parse_source(entry.lang, &bytes) else {
                continue;
            };
            for sym in symbols {
                if !is_usable_ident(&sym.name) {
                    continue;
                }
                name_files
                    .entry(sym.name.clone())
                    .or_default()
                    .insert(rel.clone());
            }
        }

        let mut names: Vec<String> = name_files
            .into_iter()
            .filter(|(_, files)| files.len() <= MAX_DEFINING_FILES)
            .map(|(n, _)| n)
            .collect();
        names.sort();
        names.truncate(MAX_TASKS_PER_FIXTURE);

        Ok(names
            .into_iter()
            .map(|name| GeneratedTask {
                task_id: format!("exact_{name}"),
                query: json!({ "pattern": name, "match": "exact" }),
                expected: vec![name.clone()],
                rule: self.id().to_string(),
                rationale: format!("exact-match MRR target for `{name}`"),
            })
            .collect())
    }
}

fn is_usable_ident(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    if name.starts_with("__") && name.ends_with("__") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
}
