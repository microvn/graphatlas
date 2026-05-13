//! Auto-GT generation — tree-sitter scan of a fixture produces ground-truth
//! tasks via AST-level rules (no tool resolution involved). Each rule is a
//! structural pattern that says "this AST shape is a hard case; here's the
//! expected result a retriever should produce". Tools are scored against
//! the rule, not against a ground-truth-of-the-world — this is honest and
//! documented (see bench methodology §Auto-GT rules).
//!
//! Rule registry:
//!   - H1 polymorphism (Python/TS class method override) → `h1_polymorphism`
//!   - H5 TS re-export chain → `h5_reexport`
//!   Others (H2 decorator, H3 getattr, H4 monkey-patch) deferred.
//!
//! ## Anti-tautology policy (M3 bench S-002 AS-005)
//!
//! New rule files (`crates/ga-bench/src/gt_gen/h<x>_*.rs`) MUST NOT import
//! `ga_query::{dead_code, callers, rename_safety, architecture, risk,
//! minimal_context}` analysis types — using them would score graphatlas
//! against itself. Allowed substrate: `ga_parser`, `ga_index` / `ga_store`,
//! and `ga_query::common` helpers (and `ga_query::import_resolve` for
//! cross-file resolution that pre-dates the analysis surface).
//!
//! Each new rule file should open with a module doc block that names the
//! policy, e.g.:
//!
//! ```text
//! //! ## Anti-tautology policy
//! //! This rule must NOT import `ga_query::{dead_code, callers, ...}`
//! //! analysis types. Allowed: `ga_parser`, `ga_index`, `ga_query::common`.
//! //! See spec §C1.
//! ```
//!
//! CI enforcement: `scripts/check-anti-tautology.sh` greps for forbidden
//! imports on every PR (build-time lint hardening deferred to Phase 3).

pub mod h1_polymorphism;
pub mod h1_text;
pub mod h2_callees_text;
pub mod h3_symbols_exact;
pub mod h4_file_summary_basic;
pub mod h5_reexport;
pub mod h5_text;
pub mod ha_import_edge;
pub mod hd_ast;
pub mod hh_gitmine;
pub mod hmc_gitmine;
pub mod hr_text;
pub mod hrn_static;

use crate::m1_ground_truth::{M1GroundTruth, M1Task, EXPECTED_SCHEMA_VERSION};
use crate::BenchError;
use ga_index::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Output of a single rule's scan. Convertible to a bench-ready [`M1Task`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTask {
    pub task_id: String,
    pub query: Value,
    pub expected: Vec<String>,
    /// Identifier of the rule that generated this task (e.g. `"H1-polymorphism"`).
    pub rule: String,
    /// Short free-text explaining why the rule matched — helps humans verify.
    pub rationale: String,
}

impl From<GeneratedTask> for M1Task {
    fn from(g: GeneratedTask) -> Self {
        // Keep rule + rationale inside query so they survive the schema
        // round-trip. Bench runner ignores extra keys.
        let mut q = g.query;
        if let Some(obj) = q.as_object_mut() {
            obj.insert("__rule".to_string(), Value::String(g.rule));
            obj.insert("__rationale".to_string(), Value::String(g.rationale));
        }
        M1Task {
            task_id: g.task_id,
            query: q,
            expected: g.expected,
        }
    }
}

pub trait GtRule {
    /// Stable identifier (e.g. `"H1-polymorphism"`). Goes into each task's
    /// `rule` field so a generated JSON is self-describing.
    fn id(&self) -> &str;

    /// Which UC this rule targets (`"callers"`, `"importers"`, …).
    fn uc(&self) -> &str;

    /// Run the scan. Errors bubble up; empty Vec is legitimate (rule didn't
    /// match anything in this fixture — not a failure).
    fn scan(&self, store: &Store, fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError>;

    /// AS-013.T1 — single source of truth for policy-bias caveats. The
    /// leaderboard renderer reads this once and prints it in the header.
    /// Default empty so existing rules don't need to retrofit the method;
    /// new rules (Hmc-budget, Hd-ast, Hrn-static, Ha-import-edge) override
    /// with substantive prose naming the rule's known biases.
    fn policy_bias(&self) -> &str {
        ""
    }
}

/// Run every rule that targets `uc`, flatten results, emit a full
/// [`M1GroundTruth`] JSON. Caller writes to disk.
pub fn generate_gt(
    uc: &str,
    fixture: &str,
    store: &Store,
    fixture_dir: &Path,
    rules: &[Box<dyn GtRule>],
) -> Result<M1GroundTruth, BenchError> {
    let mut tasks: Vec<M1Task> = Vec::new();
    for rule in rules {
        if rule.uc() != uc {
            continue;
        }
        let generated = rule.scan(store, fixture_dir)?;
        tasks.extend(generated.into_iter().map(M1Task::from));
    }
    Ok(M1GroundTruth {
        schema_version: EXPECTED_SCHEMA_VERSION,
        uc: uc.to_string(),
        fixture: fixture.to_string(),
        tasks,
    })
}

/// Default rule registry. H1-text is the canonical rule for `callers` UC
/// (raw-AST, unbiased). H1-polymorphism (graph-query variant) kept for
/// backward-compat testing but NOT in default set — it scored GA 1.0
/// tautologically and should not be used for real benchmarks.
pub fn default_rules() -> Vec<Box<dyn GtRule>> {
    default_rules_with(true)
}

/// Explicit-tests variant. Pass `exclude_tests=false` to include test files
/// as callers in generated GT — useful when benchmarking against codebases
/// where tests ARE the main caller surface.
pub fn default_rules_with(exclude_tests: bool) -> Vec<Box<dyn GtRule>> {
    vec![
        Box::new(h1_text::H1Text { exclude_tests }),
        Box::new(h2_callees_text::H2CalleesText { exclude_tests }),
        Box::new(h3_symbols_exact::H3SymbolsExact),
        Box::new(h4_file_summary_basic::H4FileSummaryBasic { exclude_tests }),
        Box::new(h5_text::H5Text),
    ]
}

/// Convenience: serialize an M1GroundTruth to pretty JSON.
pub fn to_pretty_json(gt: &M1GroundTruth) -> Result<String, BenchError> {
    serde_json::to_string_pretty(gt)
        .map_err(|e| BenchError::Other(anyhow::anyhow!("serialize gt: {e}")))
}
