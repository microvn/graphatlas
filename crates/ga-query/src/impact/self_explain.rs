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
use ga_core::{Error, Lang, Result};
use ga_index::Store;

/// CORE-3 (2026-05-22) — extract file extension and resolve to a Lang for
/// cross-language gating. Returns None for unknown extensions or paths
/// without extension; treat None as "any lang" so we don't downgrade
/// legitimate cross-extension paths (rare but possible in fixture data).
fn lang_of(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next()?;
    Lang::from_ext(ext)
}

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
    let guessed = guessed_only_files(store, symbol)?;

    for f in files.iter_mut() {
        let is_guessed = guessed.is_guessed(f.depth, f.reason, &f.path);
        let (relation, confidence, explanation) = classify(
            symbol, def_count, file_hint, f.depth, f.reason, &f.path, is_guessed,
        );
        f.relation_to_seed = relation.to_string();
        f.confidence = confidence;
        f.explanation = explanation;
    }
    Ok(())
}

/// Depth-1 files whose ONLY call connection to the seed is a name guess
/// (tier-3 repo-wide name match, no import confirmation) AND that have no
/// confirmed edge of any kind. Such a file's confidence is reduced when the
/// seed name is ambiguous (see `classify`). Computed once per call from the
/// graph — never per-edge inside a loop.
struct GuessedOnly {
    callers: std::collections::HashSet<String>,
    callees: std::collections::HashSet<String>,
}

impl GuessedOnly {
    fn is_guessed(&self, depth: u32, reason: ImpactReason, path: &str) -> bool {
        if depth != 1 {
            return false;
        }
        match reason {
            ImpactReason::Caller => self.callers.contains(path),
            ImpactReason::Callee => self.callees.contains(path),
            _ => false,
        }
    }
}

fn guessed_only_files(store: &Store, symbol: &str) -> Result<GuessedOnly> {
    if !common::is_safe_ident(symbol) {
        return Ok(GuessedOnly {
            callers: std::collections::HashSet::new(),
            callees: std::collections::HashSet::new(),
        });
    }
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    // Incoming (callers): files whose symbols call the seed.
    let calls_in = file_counts(
        &conn,
        &format!("MATCH (s:Symbol)-[:CALLS]->(t:Symbol) WHERE t.name = '{symbol}' AND s.kind <> 'external' RETURN s.file"),
    )?;
    let heur_in = file_counts(
        &conn,
        &format!("MATCH (s:Symbol)-[:CALLS_HEURISTIC]->(t:Symbol) WHERE t.name = '{symbol}' AND s.kind <> 'external' RETURN s.file"),
    )?;
    let refs_in = file_set(
        &conn,
        &format!("MATCH (s:Symbol)-[:REFERENCES]->(t:Symbol) WHERE t.name = '{symbol}' AND s.kind <> 'external' RETURN s.file"),
    )?;

    // Outgoing (callees): files the seed calls into.
    let calls_out = file_counts(
        &conn,
        &format!("MATCH (s:Symbol)-[:CALLS]->(t:Symbol) WHERE s.name = '{symbol}' AND t.kind <> 'external' RETURN t.file"),
    )?;
    let heur_out = file_counts(
        &conn,
        &format!("MATCH (s:Symbol)-[:CALLS_HEURISTIC]->(t:Symbol) WHERE s.name = '{symbol}' AND t.kind <> 'external' RETURN t.file"),
    )?;
    let refs_out = file_set(
        &conn,
        &format!("MATCH (s:Symbol)-[:REFERENCES]->(t:Symbol) WHERE s.name = '{symbol}' AND t.kind <> 'external' RETURN t.file"),
    )?;

    Ok(GuessedOnly {
        callers: guessed_set(&calls_in, &heur_in, &refs_in),
        callees: guessed_set(&calls_out, &heur_out, &refs_out),
    })
}

/// A file is guessed-only when every one of its CALLS edges to the seed is a
/// heuristic (name-guess) edge AND it has no value-reference edge to the seed.
/// `CALLS_HEURISTIC ⊆ CALLS`, so `calls == heur` (with heur > 0) means all of
/// the file's call edges are guesses; any non-heuristic CALLS edge or any
/// REFERENCES edge counts as a confirmed connection (confirmed wins).
fn guessed_set(
    calls: &std::collections::HashMap<String, u32>,
    heur: &std::collections::HashMap<String, u32>,
    refs: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for (file, h) in heur {
        if *h == 0 || refs.contains(file) {
            continue;
        }
        let c = calls.get(file).copied().unwrap_or(0);
        if c <= *h {
            out.insert(file.clone());
        }
    }
    out
}

fn file_counts(
    conn: &lbug::Connection<'_>,
    cypher: &str,
) -> Result<std::collections::HashMap<String, u32>> {
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("guessed-only query: {e}")))?;
    let mut m = std::collections::HashMap::new();
    for row in rs {
        if let Some(lbug::Value::String(f)) = row.into_iter().next() {
            *m.entry(f).or_insert(0) += 1;
        }
    }
    Ok(m)
}

fn file_set(
    conn: &lbug::Connection<'_>,
    cypher: &str,
) -> Result<std::collections::HashSet<String>> {
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("guessed-only ref query: {e}")))?;
    let mut s = std::collections::HashSet::new();
    for row in rs {
        if let Some(lbug::Value::String(f)) = row.into_iter().next() {
            s.insert(f);
        }
    }
    Ok(s)
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
        // Co-change rows (the only rows enrich_remaining touches) have no call
        // edge to the seed → never a name guess → guessed = false.
        let (relation, confidence, explanation) = classify(
            symbol, def_count, file_hint, f.depth, f.reason, &f.path, false,
        );
        f.relation_to_seed = relation.to_string();
        f.confidence = confidence;
        f.explanation = explanation;
    }
    Ok(())
}

/// Relation/confidence for a depth-1 direct edge that exists ONLY because the
/// indexer matched a bare callee name repo-wide (a name guess) to a seed whose
/// name is ambiguous (defined in >1 place) — so the guess may point at the
/// wrong definition. Distinct token from `shares_function_name` (seed
/// polymorphism) so a consumer can tell the two causes apart.
fn guessed_match_result(symbol: &str) -> (&'static str, f32, String) {
    (
        "name_guess",
        0.6,
        format!(
            "Reached `{symbol}` only by matching a bare name, and `{symbol}` is defined in \
             multiple places — this may be the wrong target. Verify before treating it as a \
             real dependency."
        ),
    )
}

fn classify(
    symbol: &str,
    def_count: usize,
    file_hint: Option<&str>,
    depth: u32,
    reason: ImpactReason,
    path: &str,
    guessed: bool,
) -> (&'static str, f32, String) {
    // CORE-3 (2026-05-22) — cross-language gate for direct-call relations.
    // When `file_hint` is set we know the seed's file (and therefore its
    // language). If `path`'s extension resolves to a different Lang AND the
    // proposed relation is a direct call edge, the indexer's name-only join
    // produced a false cross-lang match (e.g. TS `Date.now()` matching PHP
    // `TestClock::now()`). Downgrade to `shares_function_name` + conf 0.5.
    let cross_lang_collision = match (file_hint, depth, reason) {
        (Some(hint), 1, ImpactReason::Caller) | (Some(hint), 1, ImpactReason::Callee) => {
            match (lang_of(hint), lang_of(path)) {
                (Some(seed_lang), Some(file_lang)) => seed_lang != file_lang,
                _ => false,
            }
        }
        _ => false,
    };
    if cross_lang_collision {
        return (
            "shares_function_name",
            0.5,
            format!(
                "Different language than the seed file — likely a name collision \
                 (indexer matches CALLS edges by method name across languages). \
                 Verify before treating as a real dependency."
            ),
        );
    }

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
        (1, ImpactReason::Caller) => {
            if guessed && def_count > 1 {
                guessed_match_result(symbol)
            } else {
                (
                    "called_by_seed_directly",
                    1.0,
                    format!("Calls or references `{symbol}` directly."),
                )
            }
        }
        (1, ImpactReason::Callee) => {
            if guessed && def_count > 1 {
                guessed_match_result(symbol)
            } else {
                (
                    "calls_seed_directly",
                    1.0,
                    format!("`{symbol}` calls or references something defined in this file."),
                )
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // S-001 — depth-1 confidence reflects name-guess + seed ambiguity.
    // classify(symbol, def_count, file_hint, depth, reason, path, guessed).
    // Coverage (spec IDs as literal tokens for the Coverage Gate):
    // AS-001, AS-002, AS-003, AS-004, AS-008, AS-009.
    // Constraints covered transitively by these AS (CC5 mapping):
    // C-001 (AS-001, AS-002), C-002 (AS-001, AS-002, AS-008),
    // C-003 (AS-003, AS-004, AS-009), C-004 (AS-009).

    #[test]
    fn as_001_confirmed_direct_caller_stays_full_confidence() {
        let (rel, conf, _) = classify(
            "parse_config",
            1,
            None,
            1,
            ImpactReason::Caller,
            "loader.py",
            false,
        );
        assert!(
            (conf - 1.0).abs() < 1e-6,
            "confirmed depth-1 caller → 1.0, got {conf}"
        );
        assert_eq!(rel, "called_by_seed_directly");
    }

    #[test]
    fn as_002_guessed_edge_to_ambiguous_seed_is_downgraded() {
        let (rel, conf, expl) = classify(
            "flush",
            3,
            Some("seed.py"),
            1,
            ImpactReason::Caller,
            "worker.py",
            true,
        );
        assert!(
            (conf - 0.6).abs() < 1e-6,
            "guessed + ambiguous → 0.6, got {conf}"
        );
        assert_eq!(
            rel, "name_guess",
            "must use a distinct token, not shares_function_name"
        );
        assert!(!expl.is_empty());
    }

    #[test]
    fn as_003_confirmed_edge_to_ambiguous_seed_not_downgraded() {
        // Min-depth confirmed edge wins even when the seed name is ambiguous
        // and a deeper guessed path exists: guessed=false at this depth-1 row.
        let (_, conf, _) = classify(
            "flush",
            3,
            Some("seed.py"),
            1,
            ImpactReason::Caller,
            "h.py",
            false,
        );
        assert!(
            (conf - 1.0).abs() < 1e-6,
            "confirmed depth-1 wins → 1.0, got {conf}"
        );
    }

    #[test]
    fn as_004_seed_file_always_full_confidence() {
        let (rel, conf, _) = classify("Router", 1, None, 0, ImpactReason::Seed, "router.go", false);
        assert!((conf - 1.0).abs() < 1e-6, "seed → 1.0, got {conf}");
        assert_eq!(rel, "changed_directly");
    }

    #[test]
    fn as_008_guessed_edge_to_unique_seed_keeps_full_confidence() {
        // def_count==1 → a name guess has only one possible target → reliable.
        let (rel, conf, _) = classify(
            "reconcile_ledger_v2",
            1,
            None,
            1,
            ImpactReason::Caller,
            "job.rs",
            true,
        );
        assert!(
            (conf - 1.0).abs() < 1e-6,
            "guessed but unique seed → 1.0, got {conf}"
        );
        assert_eq!(
            rel, "called_by_seed_directly",
            "unique-seed guess is not flagged"
        );
    }

    #[test]
    fn as_009_cochange_row_keeps_existing_confidence() {
        // Co-change has no call edge; the guess downgrade must never touch it.
        let (rel, conf, _) = classify(
            "S",
            3,
            Some("s.py"),
            1,
            ImpactReason::CoChange,
            "c.py",
            true,
        );
        assert!(
            (conf - 0.5).abs() < 1e-6,
            "co-change keeps 0.5 regardless of guessed, got {conf}"
        );
        assert_eq!(rel, "co_changes_with_seed");
    }
}
