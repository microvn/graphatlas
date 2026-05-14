//! v1.2-php S-002 AS-022 — every registered Retriever declares its
//! `supported_langs` upfront, so the bench harness can distinguish:
//!   - `[skip: lang_unsupported]` (declared NOT supported — honest empty)
//!   - `0.00 (error)`               (declared supported, crashed at query)
//!   - `0.00 (zero hits)`           (declared supported, ran, found nothing)
//!
//! Before this AS, all three outcomes looked identical in the leaderboard.
//! Methodology.md §Fairness audit log Iteration 4 documented this trap.

use ga_bench::retriever::Retriever;
use ga_core::Lang;

// Cover each retriever currently registered in ga-bench. Instantiate via
// constructors so the test exercises the real impl tables (not a stub).

#[test]
fn ga_retriever_supports_full_lang_set() {
    let r = ga_bench::retrievers::ga::GaRetriever::new(std::path::PathBuf::from(
        "/tmp/ga_supports_test",
    ));
    let supported = r.supported_langs();
    assert!(
        !supported.is_empty(),
        "GA retriever supported_langs must be non-empty"
    );
    // GA is the reference engine — it claims all langs the indexer indexes.
    for lang in [Lang::Python, Lang::TypeScript, Lang::Php] {
        assert!(
            supported.contains(&lang),
            "GA retriever must declare {lang:?} support: {supported:?}"
        );
    }
}

#[test]
fn default_supported_langs_is_lang_all() {
    // The trait default returns Lang::ALL. Any retriever that does NOT
    // override is treated as "claims all langs" — existing behavior preserved.
    struct StubRetriever;
    impl Retriever for StubRetriever {
        fn name(&self) -> &str {
            "stub"
        }
        fn query(
            &mut self,
            _uc: &str,
            _query: &serde_json::Value,
        ) -> Result<Vec<String>, ga_bench::BenchError> {
            Ok(Vec::new())
        }
    }
    let stub = StubRetriever;
    assert_eq!(
        stub.supported_langs(),
        Lang::ALL,
        "default supported_langs must return Lang::ALL (backward-compat)"
    );
}

#[test]
fn retriever_can_narrow_supported_langs() {
    // Verify the trait method is override-able. Future retrievers wanting
    // honest disclosure can declare narrower coverage.
    struct PhpOnlyRetriever;
    const PHP_ONLY: &[Lang] = &[Lang::Php];
    impl Retriever for PhpOnlyRetriever {
        fn name(&self) -> &str {
            "php-only"
        }
        fn supported_langs(&self) -> &'static [Lang] {
            PHP_ONLY
        }
        fn query(
            &mut self,
            _uc: &str,
            _query: &serde_json::Value,
        ) -> Result<Vec<String>, ga_bench::BenchError> {
            Ok(Vec::new())
        }
    }
    let r = PhpOnlyRetriever;
    assert_eq!(r.supported_langs(), &[Lang::Php]);
    assert!(
        !r.supported_langs().contains(&Lang::Python),
        "narrowed retriever should not claim Python"
    );
}

#[test]
fn supports_lang_helper_returns_bool() {
    // Common consumer pattern: check if a specific Lang is in the slice.
    // No helper method needed — `slice.contains(&Lang::X)` is the canonical idiom.
    let r =
        ga_bench::retrievers::ga::GaRetriever::new(std::path::PathBuf::from("/tmp/ga_lang_check"));
    assert!(r.supported_langs().contains(&Lang::Php));
    // Iteration is cheap (≤10 entries) so no need for HashSet.
}
