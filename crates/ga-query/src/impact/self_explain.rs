//! Self-explaining enrichment for `ImpactedFile`. Populates `confidence`,
//! `relation_to_seed`, and `explanation` so an LLM consumer that has never
//! seen GA's spec can read the response and understand why each file is in
//! the list.
//!
//! Vocabulary chosen to be universal — no GA-internal taxonomy
//! (`PolymorphicDef`, `KinshipViaCallee`, …) leaks. Tokens are CS basics
//! every LLM already knows: "changed directly", "shares function name",
//! "calls seed directly", "called by seed directly", "shared dependency",
//! "sibling in same class", "co-changes with seed".
//!
//! Bench-safety: this layer adds fields ONLY. It never adds, removes, or
//! reorders entries in `impacted_files`. Bench retrievers extract `path`
//! and the score is identical to before this enrichment existed.

use super::types::{ImpactReason, ImpactedFile};
use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;

/// Decorate each `ImpactedFile` with confidence + relation_to_seed +
/// explanation. `file_hint` is the user-supplied narrowing hint (Tools-C11)
/// — when present, it identifies the file the user is actually changing.
pub(super) fn enrich(
    files: &mut [ImpactedFile],
    store: &Store,
    symbol: &str,
    file_hint: Option<&str>,
) -> Result<()> {
    let def_count = count_defs(store, symbol)?;

    for f in files.iter_mut() {
        let (relation, confidence, explanation) =
            classify(symbol, def_count, file_hint, f.depth, f.reason, &f.path);
        f.relation_to_seed = relation.to_string();
        f.confidence = confidence;
        f.explanation = explanation;
    }
    Ok(())
}

/// Backfill self-explain fields on rows whose `relation_to_seed` is still
/// the empty default — typically EXP-M2-11 co-change entries appended after
/// the main enrich pass. Skips already-classified rows so we don't pay an
/// extra `count_defs` traversal cost on hot paths.
pub(super) fn enrich_remaining(
    files: &mut [ImpactedFile],
    store: &Store,
    symbol: &str,
    file_hint: Option<&str>,
) -> Result<()> {
    if !files.iter().any(|f| f.relation_to_seed.is_empty()) {
        return Ok(());
    }
    let def_count = count_defs(store, symbol)?;
    for f in files.iter_mut() {
        if !f.relation_to_seed.is_empty() {
            continue;
        }
        let (relation, confidence, explanation) =
            classify(symbol, def_count, file_hint, f.depth, f.reason, &f.path);
        f.relation_to_seed = relation.to_string();
        f.confidence = confidence;
        f.explanation = explanation;
    }
    Ok(())
}

fn classify(
    symbol: &str,
    def_count: usize,
    file_hint: Option<&str>,
    depth: u32,
    reason: ImpactReason,
    path: &str,
) -> (&'static str, f32, String) {
    match (depth, reason) {
        (0, ImpactReason::Seed) => {
            // Tools-C11: file_hint match → 1.0; polymorphic non-hint def → 0.6.
            let hint_match = matches!(file_hint, Some(h) if h == path);
            if def_count <= 1 || hint_match {
                (
                    "changed_directly",
                    1.0,
                    format!("This file defines `{symbol}` and is the change target."),
                )
            } else {
                (
                    "shares_function_name",
                    0.6,
                    format!(
                        "Defines a function named `{symbol}` separately from the change target. \
                         Likely duplicated logic — verify whether the rename/refactor should apply here too."
                    ),
                )
            }
        }
        (1, ImpactReason::Caller) => (
            "called_by_seed_directly",
            1.0,
            format!("Calls or references `{symbol}` directly."),
        ),
        (1, ImpactReason::Callee) => (
            "calls_seed_directly",
            1.0,
            format!("`{symbol}` calls or references something defined in this file."),
        ),
        (2, ImpactReason::Callee) => (
            "sibling_in_same_class",
            0.7,
            format!(
                "Reached through a sibling method on the same class as `{symbol}`. \
                 Indirect — review only if the class contract changes."
            ),
        ),
        (_, ImpactReason::Caller | ImpactReason::Callee) => (
            "shared_dependency",
            0.4,
            format!(
                "Reached only through a chain of {} hops via shared callees/callers. \
                 Likely shares a dependency with `{symbol}` rather than being directly impacted.",
                depth
            ),
        ),
        (_, ImpactReason::CoChange) => (
            "co_changes_with_seed",
            0.5,
            format!(
                "Historically modified together with the seed file across recent commits, \
                 even though no direct call edge exists."
            ),
        ),
        (_, ImpactReason::Seed) => {
            // Defensive: depth>0 with reason=Seed shouldn't happen, but if it
            // does we'd rather emit something self-evident than panic.
            ("changed_directly", 1.0, format!("Defines `{symbol}`."))
        }
    }
}

fn count_defs(store: &Store, symbol: &str) -> Result<usize> {
    if !common::is_safe_ident(symbol) {
        return Ok(0);
    }
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{symbol}' AND s.kind <> 'external' \
         RETURN count(DISTINCT s.file)"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("count_defs: {e}")))?;
    for row in rs {
        for val in row.into_iter() {
            if let lbug::Value::Int64(n) = val {
                return Ok(n as usize);
            }
        }
    }
    Ok(0)
}
