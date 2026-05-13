//! H1-text — raw-AST polymorphism-caller rule. Unbiased replacement for H1
//! (which built expected lists via `ga_query::callers`, tautological for GA).
//!
//! ## Policy (name-based dynamic-lang semantics)
//!
//! For each symbol `M` appearing as a callee in ≥2 distinct call sites:
//! ```text
//! expected[M] = { enclosing_fn_name  |  call_site has text "M"  in its AST }
//! ```
//!
//! No graph resolution. No type info. Every tool scored against the same
//! structural signal — tree-sitter's raw call-site extraction. Tools whose
//! policy matches this rule (return every same-name caller regardless of
//! receiver type) score near 1.0; tools that filter by type-info score
//! lower by construction.
//!
//! Documented caveat: this biases TOWARD dynamic-lang semantics. Rust / TS
//! tools that use type information to narrow callers will score lower —
//! that's the rule's choice, not a bug.
//!
//! ## Filters
//!
//! - Dunder methods (`__init__`, `__str__`) — too common, drown real signal.
//! - Callee name length < 3 — too generic (`f`, `cb`).
//! - Expected set size > 30 — symbol is too generic to differentiate tools.
//! - Fewer than 2 call sites — no signal.
//! - When `exclude_tests=true`, test-file call sites are filtered out before
//!   the expected set is computed. Retriever outputs are filtered the same
//!   way at scoring time (handled in runner, not here).

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_core::Lang;
use ga_index::Store;
use ga_parser::calls::extract_calls;
use ga_parser::walk::walk_repo;
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub struct H1Text {
    pub exclude_tests: bool,
}

impl Default for H1Text {
    fn default() -> Self {
        Self {
            exclude_tests: true,
        }
    }
}

impl GtRule for H1Text {
    fn id(&self) -> &str {
        "H1-text"
    }
    fn uc(&self) -> &str {
        "callers"
    }

    /// `store` is unused — this rule deliberately avoids graph queries.
    /// Accepted as a parameter to match the trait; could be `_store` but
    /// the signature is shared across all rules.
    fn scan(&self, _store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        // 1. Walk repo, for each source file run extract_calls.
        let report = walk_repo(fixture_dir)
            .map_err(|e| BenchError::Other(anyhow::anyhow!("walk_repo: {e}")))?;

        // Build: callee_name -> set of (enclosing_fn, file)
        // Also: callee_name -> total sites seen (even without enclosing)
        let mut callers_by_callee: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut site_count: HashMap<String, usize> = HashMap::new();

        for entry in &report.entries {
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
                if !is_clean_name(&c.callee_name, entry.lang) {
                    continue;
                }
                *site_count.entry(c.callee_name.clone()).or_insert(0) += 1;
                if let Some(enclosing) = c.enclosing_symbol {
                    if is_clean_name(&enclosing, entry.lang) {
                        callers_by_callee
                            .entry(c.callee_name)
                            .or_default()
                            .insert(enclosing);
                    }
                }
            }
        }

        // 2. Emit tasks for callees with enough signal.
        let mut out = Vec::new();
        for (callee, callers) in &callers_by_callee {
            if site_count.get(callee).copied().unwrap_or(0) < 2 {
                continue; // no polymorphic-like signal
            }
            if callers.len() > 30 {
                continue; // too generic (e.g. `push`, `write`)
            }
            if callers.is_empty() {
                continue;
            }
            let expected: Vec<String> = callers.iter().cloned().collect();
            let task_id = format!("textmatch_{}", callee);
            out.push(GeneratedTask {
                task_id,
                query: json!({ "symbol": callee }),
                expected,
                rule: self.id().to_string(),
                rationale: format!(
                    "{} sites reference `{}` across {} enclosing function(s); raw-AST text match",
                    site_count.get(callee).unwrap_or(&0),
                    callee,
                    callers.len()
                ),
            });
        }
        Ok(out)
    }
}

fn is_clean_name(s: &str, lang: Lang) -> bool {
    if s.is_empty() || s.len() < 3 || s.len() > 512 {
        return false;
    }
    if is_dunder(s) {
        return false;
    }
    // Stopwords / language noise that create false positives.
    if matches!(s, "new" | "self" | "this" | "super" | "clone") {
        return false;
    }
    // Language-specific common names that drown real signal.
    if matches!(lang, Lang::Rust)
        && matches!(
            s,
            "into" | "from" | "as_ref" | "as_mut" | "unwrap" | "expect"
        )
    {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
}

fn is_dunder(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__") && name.len() >= 5
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`. The previous local `pub fn`
// covered only Python/TS/JS/Go/Rust suffix patterns and was the
// §4.2.5 STALE site for GT generation rule H1 (text-callees) —
// caused mockito Java fixture H1 GT to mis-classify `*Test.java`.
pub use ga_query::common::is_test_path;
