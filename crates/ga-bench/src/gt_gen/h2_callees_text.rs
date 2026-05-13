//! H2-text — raw-AST callees rule. Unbiased GT for `callees` UC.
//!
//! For each enclosing function `F`, expected callees = set of callee names
//! in call-sites within F's body (via `ga_parser::extract_calls`). No graph
//! resolution.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use ga_parser::{extract_calls, walk::walk_repo};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub struct H2CalleesText {
    pub exclude_tests: bool,
}

impl Default for H2CalleesText {
    fn default() -> Self {
        Self {
            exclude_tests: true,
        }
    }
}

impl GtRule for H2CalleesText {
    fn id(&self) -> &str {
        "H2-callees-text"
    }
    fn uc(&self) -> &str {
        "callees"
    }

    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let walk = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // enclosing_fn → (first-seen-file, set<callee>)
        let mut per_fn: HashMap<String, (String, BTreeSet<String>)> = HashMap::new();

        for entry in &walk.entries {
            let rel = entry.rel_path.to_string_lossy().into_owned();
            if self.exclude_tests && is_test_path(&rel) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&entry.abs_path) else {
                continue;
            };
            let Ok(calls) = extract_calls(entry.lang, &bytes) else {
                continue;
            };
            for c in calls {
                let Some(enclosing) = c.enclosing_symbol.as_deref() else {
                    continue;
                };
                if !is_usable_ident(enclosing) || !is_usable_ident(&c.callee_name) {
                    continue;
                }
                per_fn
                    .entry(enclosing.to_string())
                    .or_insert_with(|| (rel.clone(), BTreeSet::new()))
                    .1
                    .insert(c.callee_name.clone());
            }
        }

        let mut out = Vec::new();
        for (fn_name, (file, callees)) in per_fn {
            if callees.len() < 2 || callees.len() > 30 {
                continue;
            }
            let n = callees.len();
            out.push(GeneratedTask {
                task_id: format!("textmatch_{fn_name}"),
                query: json!({ "symbol": fn_name, "file": file }),
                expected: callees.into_iter().collect(),
                rule: self.id().to_string(),
                rationale: format!("{n} callees appear inside `{fn_name}`; raw-AST match"),
            });
        }
        out.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        Ok(out)
    }
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`. Local copy was correct for
// Java/Kotlin/C#/Ruby suffixes but missed *IT.kt + KMP multi-target
// (commonTest / jvmTest / androidTest / etc.) per S-002 lock-step audit.
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
