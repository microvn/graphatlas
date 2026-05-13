//! v1.1-M4 S-004a — Ruby LanguageSpec skeleton contract.
//!
//! Pin the **registration-level** invariants for `Lang::Ruby` independent
//! of per-AS extraction logic (which lands in S-004b/c). These tests flip
//! the inverse contract previously held by `language_spec_unknown.rs`
//! (Ruby was the last "spec not registered" probe) and replace it with
//! positive assertions:
//!
//!   - `ParserPool::new()` registers a `LanguageSpec` for `Lang::Ruby`
//!   - symbol / call / extends node-kind lists cover the baseline kinds
//!     S-004b/c will depend on
//!   - import_node_kinds is intentionally empty (Ruby has no static
//!     import statement; require/require_relative are runtime calls)
//!   - the four public extractors return `Ok` on a Ruby source — concrete
//!     extracted-record shapes belong to S-004b/c per-AS tests
//!   - `family()` reports `LangFamily::DynamicScripting` (Python/Ruby/PHP/Lua)

use ga_core::Lang;
use ga_parser::{
    extract_calls, extract_extends, extract_imports, extract_references, LangFamily, ParserPool,
};

const RUBY_SOURCE: &[u8] = b"\
module App\n\
  class User < Base\n\
    def initialize(name)\n\
      check(name)\n\
    end\n\
    def self.find(id)\n\
      Base.lookup(id)\n\
    end\n\
  end\n\
end\n";

#[test]
fn registers_rubylang_in_pool() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Ruby);
    assert!(
        spec.is_some(),
        "S-004a: ParserPool::new() must register a LanguageSpec for Lang::Ruby"
    );
    assert_eq!(spec.unwrap().lang(), Lang::Ruby);
}

#[test]
fn ruby_family_is_dynamic_scripting() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Ruby).expect("Ruby registered");
    assert_eq!(
        spec.family(),
        LangFamily::DynamicScripting,
        "S-004a: Ruby family must be DynamicScripting (groups Python/Ruby/PHP/Lua duck-typed langs)"
    );
}

#[test]
fn ruby_node_kind_lists_cover_baseline() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Ruby).expect("Ruby registered");
    assert!(!spec.symbol_node_kinds().is_empty(), "symbol_node_kinds");
    assert!(!spec.call_node_kinds().is_empty(), "call_node_kinds");
    assert!(!spec.extends_node_kinds().is_empty(), "extends_node_kinds");
    // Ruby has NO static import statement — require/require_relative are
    // runtime method calls (parsed as `call` nodes). Empty list is correct.
    assert!(
        spec.import_node_kinds().is_empty(),
        "import_node_kinds must be empty for Ruby (no static imports — \
         require is a runtime call, see langs/ruby.rs IMPORTS)"
    );
}

#[test]
fn ruby_node_kinds_include_baseline_set() {
    // Sanity floor: the spec promises Ruby symbols include classes,
    // modules, methods, and singleton-methods. Calls cover the unified
    // `call` kind. Extends cover class<superclass declarations.
    // Stronger AS-016 dynamic-drift coverage lives in grammar_drift.rs.
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Ruby).expect("Ruby registered");
    for required in &["class", "module", "method", "singleton_method"] {
        assert!(
            spec.symbol_node_kinds().contains(required),
            "symbol_node_kinds must include `{required}`"
        );
    }
    assert!(
        spec.call_node_kinds().contains(&"call"),
        "call_node_kinds must include `call` (tree-sitter-ruby unifies all \
         invocation forms under this kind — verified via AST probe)"
    );
    assert!(
        spec.extends_node_kinds().contains(&"class"),
        "extends_node_kinds must include `class` (Ruby class<superclass \
         field hangs off the `class` declaration node)"
    );
}

#[test]
fn extract_calls_on_ruby_source_returns_ok() {
    let result = extract_calls(Lang::Ruby, RUBY_SOURCE);
    assert!(
        result.is_ok(),
        "S-004a: extract_calls must return Ok on Ruby source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_references_on_ruby_source_returns_ok() {
    let result = extract_references(Lang::Ruby, RUBY_SOURCE);
    assert!(
        result.is_ok(),
        "S-004a: extract_references must return Ok on Ruby source, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_extends_on_ruby_source_returns_ok() {
    let result = extract_extends(Lang::Ruby, RUBY_SOURCE);
    assert!(
        result.is_ok(),
        "S-004a: extract_extends must return Ok on Ruby source, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_imports_on_ruby_source_returns_ok() {
    // Ruby has no static imports — but the public extractor must still
    // return Ok (with an empty Vec). A registered spec with empty
    // import_node_kinds means the engine walks the AST, finds no matches,
    // and returns Ok([]).
    let result = extract_imports(Lang::Ruby, RUBY_SOURCE);
    assert!(
        result.is_ok(),
        "S-004a: extract_imports must return Ok([]) on Ruby source (no static imports), got: {:?}",
        result.err()
    );
    assert!(
        result.unwrap().is_empty(),
        "Ruby import_node_kinds is empty → extract_imports must return empty Vec"
    );
}

#[test]
fn extract_calls_on_empty_ruby_source_returns_ok_empty() {
    // Edge: empty input must still parse + return empty Vec, not error.
    let result = extract_calls(Lang::Ruby, b"");
    assert!(
        result.is_ok(),
        "edge: empty Ruby source must parse cleanly, got: {:?}",
        result.err()
    );
    assert!(result.unwrap().is_empty(), "empty source → no calls");
}

#[test]
fn extract_calls_does_not_panic_on_garbage_bytes() {
    // R12 contract: parser must NEVER panic, even on garbage byte streams.
    // The skeleton's extractors should inherit this from the engine's
    // default traversal — defense-in-depth check.
    let garbage: &[u8] = &[0xff, 0xfe, 0xfd, 0x00, 0xff, 0xfe, 0xfd, 0x00];
    let result = std::panic::catch_unwind(|| extract_calls(Lang::Ruby, garbage));
    assert!(
        result.is_ok(),
        "extract_calls panicked on garbage Ruby bytes — R12 violated"
    );
}
