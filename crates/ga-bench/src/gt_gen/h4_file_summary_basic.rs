//! H4-basic — raw-AST file_summary rule.
//!
//! For each source file, emit `ga_file_summary {path: file}` task; expected
//! = set of defined symbol names (from `ga_parser::parse_source`). F1-scored.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::{parse_source, walk::walk_repo};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;

pub struct H4FileSummaryBasic {
    pub exclude_tests: bool,
}

impl Default for H4FileSummaryBasic {
    fn default() -> Self {
        Self {
            exclude_tests: true,
        }
    }
}

impl GtRule for H4FileSummaryBasic {
    fn id(&self) -> &str {
        "H4-file-summary-basic"
    }
    fn uc(&self) -> &str {
        "file_summary"
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let walk = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;
        let mut out: Vec<GeneratedTask> = Vec::new();

        for entry in &walk.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            if self.exclude_tests && is_test_path(&rel) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&entry.abs_path) else {
                continue;
            };
            let Ok(symbols) = parse_source(entry.lang, &bytes) else {
                continue;
            };
            let names: BTreeSet<String> = symbols
                .into_iter()
                .map(|s| s.name)
                .filter(|n| is_usable_ident(n))
                .collect();
            if names.is_empty() || names.len() > 30 {
                continue;
            }
            out.push(GeneratedTask {
                task_id: format!("summary_{}", rel.replace(['/', '.'], "_"),),
                query: json!({ "path": rel }),
                expected: names.into_iter().collect(),
                rule: self.id().to_string(),
                rationale: format!("defined symbols in `{rel}`"),
            });
        }

        out.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        Ok(out)
    }
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`.
use ga_query::common::is_test_path;

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
