//! Retrieval ensemble signals — git-log-based heuristics used to re-rank
//! or filter candidate file sets post-primary-retrieval.
//!
//! Ported from `src/adapters/{co-change,hub-penalty}.ts`. These don't run
//! inside `ga_impact` (which must stay ≤500ms, no subprocess) — they live
//! here for bench ensemble retrievers that already tolerate git calls.

pub mod co_change;
pub mod hub_penalty;
pub mod importers;
