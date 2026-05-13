//! v1.1-M4 (S-005a D6) — AS-017: unknown language returns typed error.
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-017):
//!   Given: caller passes `Lang::<Variant>` whose enum variant exists post-D1
//!     but no `LanguageSpec` impl is registered in `ParserPool`.
//!   When:  caller invokes extract_calls / extract_references /
//!     extract_extends / extract_imports / parse_source.
//!   Then:  each returns Err(ga_core::Error::Other(anyhow!(...))) with
//!     message containing the lang name. No panic. No silent pass.
//!
//! **Post-S-004a status**: ALL nine v1.1 langs are now registered
//! (Python / TypeScript / JavaScript / Go / Rust / Java / Kotlin / CSharp /
//! Ruby). There is no longer a Lang variant whose `LanguageSpec` is
//! missing in `ParserPool::new()`, so the per-extractor "returns Err for
//! unregistered lang" tests have no target to probe.
//!
//! The AS-017 error-path code in `lib.rs::parse_source` (and the four
//! `extract_*` functions) is retained for forward compatibility — when v2+
//! adds Swift / PHP / Elixir / etc. as `Lang` variants ahead of their
//! `LanguageSpec` impls, those transitional langs will hit the same error
//! path and the next-shipped-lang's skeleton story (S-00X-a equivalent)
//! will re-introduce a probe test against them.
//!
//! What this file pins NOW: registration completeness — every Lang variant
//! that exists has a registered spec, no silent gaps.

use ga_core::Lang;
use ga_parser::ParserPool;

#[test]
fn all_v1_plus_phase_c_langs_registered() {
    // Sanity: every Lang variant from `ga_core::Lang` has a registered
    // LanguageSpec in ParserPool::new(). If a new Lang variant lands
    // without a corresponding `Box::new(langs::*::*Lang)` registration
    // in `ParserPool::new()`, this guard turns red.
    let pool = ParserPool::new();
    for &lang in Lang::ALL {
        assert!(
            pool.spec_for(lang).is_some(),
            "v1.1-M4 invariant: every Lang variant must be registered. \
             Missing: {lang:?}. Add the spec impl + register in ParserPool::new()."
        );
    }
}

#[test]
fn registered_lang_count_matches_enum_size() {
    // Belt-and-suspenders: the `registered_langs()` accessor returns one
    // entry per registered spec. After S-004a (Ruby) the count must equal
    // the total Lang variant count (9 = 5 v1 + 4 v1.1-M4).
    let pool = ParserPool::new();
    assert_eq!(
        pool.registered_langs().len(),
        Lang::ALL.len(),
        "registered_langs count drifted from Lang::ALL — a variant was \
         added to the enum without registration in ParserPool::new()"
    );
}
