//! Git-subprocess signals used by impact pipeline. Breaks the previous
//! "no subprocess in ga-query" guard — accepted per EXP-M2-11 to lift
//! `blast_radius_coverage` + `adj_prec` (LLM agent utility). Opt-out
//! via `ImpactRequest.include_co_change_importers=false` for queries
//! that need tight 500ms p95 budget.
//!
//! Modules:
//! - [`co_change`] — Phase B: files that co-change with seed in last
//!   N commits. Ported from `ga-bench/src/signals/co_change.rs`.
//! - [`importers`] — Phase A: per-language `git grep` of import sites.
//!   Ported from `ga-bench/src/signals/importers.rs`.
//! - [`co_change_importers`] — Phase C: intersection A ∩ B (mimics GT
//!   `should_touch_files` derivation per `extract-seeds.ts:491-538`).

pub mod co_change;
pub mod co_change_importers;
pub mod importers;
