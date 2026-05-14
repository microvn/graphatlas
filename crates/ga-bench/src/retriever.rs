//! Retriever trait — the abstraction each bench-able tool implements so the
//! runner doesn't care whether results come from in-process Rust (ga), a
//! one-shot subprocess (ripgrep), or a persistent MCP stdio child
//! (codegraphcontext / codebase-memory).
//!
//! Lifecycle:
//! 1. `setup(fixture_dir)` — called once before any query. MCP retrievers use
//!    this to spawn the server child + run pre-index commands. Native
//!    retrievers typically open a `Store` + `build_index`.
//! 2. `query(uc, q)` — one call per GT task. Returns the list of names the
//!    retriever considers the answer for this UC + query shape.
//! 3. `teardown()` — kill any long-running child, drop resources. Called on
//!    run_uc completion regardless of success.
//!
//! `query` returns an empty Vec (not an error) when the retriever has no
//! plausible answer for this UC — e.g. `ripgrep` on `callers` — so the
//! scorer still counts the task toward the retriever's pass rate without
//! crashing the whole bench.

use crate::BenchError;
use ga_core::Lang;
use serde_json::Value;
use std::path::Path;

pub trait Retriever: Send {
    /// Short identifier written into leaderboard `retriever` column (e.g.
    /// `"ga"`, `"ripgrep"`, `"codegraphcontext"`).
    fn name(&self) -> &str;

    /// v1.2-php S-002 AS-022 — declared lang coverage.
    ///
    /// Returned slice is the set of `Lang` variants this retriever indexes or
    /// natively understands. The bench harness uses this to distinguish three
    /// outcomes that previously all looked like "0.00":
    ///
    /// 1. **Not supported** — `Lang::Php ∉ supported_langs()` → harness emits
    ///    `[skip: lang_unsupported]` in the leaderboard row. Honest disable.
    /// 2. **Supported but errored** — query path crashes or returns Err →
    ///    surfaces as `0.00 (error)` in the leaderboard.
    /// 3. **Supported, ran, zero hits** — legitimate empty result → `0.00`.
    ///
    /// Default returns the full `Lang::ALL` slice so existing retrievers keep
    /// their previous "claim everything" behavior — explicit narrowing is
    /// opt-in per retriever as PHP/other-lang coverage matures.
    fn supported_langs(&self) -> &'static [Lang] {
        Lang::ALL
    }

    /// Pre-flight — build indices, spawn child processes, warm caches.
    /// Default impl no-op so retrievers that don't need setup just get it free.
    #[allow(unused_variables)]
    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        Ok(())
    }

    /// Execute one task. `uc` is the use-case id (`"callers"`, `"callees"`,
    /// `"importers"`, `"symbols"`, `"file_summary"`). `query` is the raw
    /// ground-truth task query object (shape depends on UC).
    fn query(&mut self, uc: &str, query: &Value) -> Result<Vec<String>, BenchError>;

    /// Release resources. Called even on error paths.
    fn teardown(&mut self) {}

    /// Structured impact query — returns the full file/test/route actuals
    /// so the 4-dim [`crate::score::impact_score`] can be computed without
    /// re-running the tool. Default `None` keeps retrievers that don't
    /// natively expose impact analysis out of uc=impact measurement.
    ///
    /// Only used when `uc == "impact"` and the GT carries multi-dim labels
    /// (`expected_files` / `expected_tests` / `expected_routes`).
    #[allow(unused_variables)]
    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        None
    }
}

/// Multi-dim retriever output used by uc=impact scoring.
#[derive(Debug, Clone, Default)]
pub struct ImpactActual {
    pub files: Vec<String>,
    pub tests: Vec<String>,
    pub routes: Vec<String>,
    pub transitive_completeness: u32,
    pub max_depth: u32,
}
