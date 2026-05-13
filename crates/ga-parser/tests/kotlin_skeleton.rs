//! v1.1-M4 S-002a — Kotlin LanguageSpec skeleton contract.
//!
//! Pin the **registration-level** invariants for `Lang::Kotlin` independent
//! of per-AS extraction logic (which lands in S-002b/S-002c). These tests
//! flip the inverse contract previously held by `language_spec_unknown.rs`
//! (Kotlin was "spec not registered") and replace it with positive
//! assertions:
//!
//!   - `ParserPool::new()` registers a `LanguageSpec` for `Lang::Kotlin`
//!   - all four AST node-kind lists are non-empty + cover the baseline
//!     nodes the per-AS tests in S-002b/c will depend on
//!   - the four public extractors (`extract_calls/references/extends/
//!     imports`) return `Ok` on a Kotlin source — concrete shape of
//!     extracted records belongs to S-002b/c per-AS tests
//!   - `family()` reports `LangFamily::StaticManaged` (Java/Kotlin/Scala/C#)
//!
//! The "no-spec → typed Err" path is still covered for AS-017 — the probe
//! moves to `Lang::CSharp` (still unregistered until S-003 lands), see
//! `language_spec_unknown.rs`.

use ga_core::Lang;
use ga_parser::{
    extract_calls, extract_extends, extract_imports, extract_references, LangFamily, ParserPool,
};

const KOTLIN_SOURCE: &[u8] = b"\
package com.example\n\
import com.example.util.Helper\n\
class UserService {\n\
    fun getUser(): User = User()\n\
}\n";

#[test]
fn registers_kotlinlang_in_pool() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Kotlin);
    assert!(
        spec.is_some(),
        "S-002a: ParserPool::new() must register a LanguageSpec for Lang::Kotlin"
    );
    assert_eq!(spec.unwrap().lang(), Lang::Kotlin);
}

#[test]
fn kotlin_family_is_static_managed() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Kotlin).expect("Kotlin registered");
    assert_eq!(
        spec.family(),
        LangFamily::StaticManaged,
        "S-002a: Kotlin family must be StaticManaged (groups JVM+.NET typed-managed langs)"
    );
}

#[test]
fn kotlin_node_kind_lists_non_empty() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Kotlin).expect("Kotlin registered");
    assert!(!spec.symbol_node_kinds().is_empty(), "symbol_node_kinds");
    assert!(!spec.import_node_kinds().is_empty(), "import_node_kinds");
    assert!(!spec.call_node_kinds().is_empty(), "call_node_kinds");
    assert!(!spec.extends_node_kinds().is_empty(), "extends_node_kinds");
}

#[test]
fn kotlin_node_kinds_include_baseline_set() {
    // Sanity floor: the spec promises Kotlin symbols include classes +
    // objects + functions, imports cover the Kotlin `import` node (NOT
    // `import_declaration` — distinct from Java), calls cover
    // call_expression. Stronger AS-016 dynamic-drift coverage lives in
    // grammar_drift.rs.
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Kotlin).expect("Kotlin registered");
    assert!(
        spec.symbol_node_kinds().contains(&"class_declaration"),
        "symbol_node_kinds must include `class_declaration`"
    );
    assert!(
        spec.symbol_node_kinds().contains(&"object_declaration"),
        "symbol_node_kinds must include `object_declaration` (Kotlin singleton form)"
    );
    assert!(
        spec.symbol_node_kinds().contains(&"function_declaration"),
        "symbol_node_kinds must include `function_declaration`"
    );
    assert!(
        spec.import_node_kinds().contains(&"import"),
        "import_node_kinds must include bare `import` (Kotlin grammar uses `import`, not `import_declaration`)"
    );
    assert!(
        spec.call_node_kinds().contains(&"call_expression"),
        "call_node_kinds must include `call_expression`"
    );
}

#[test]
fn extract_calls_on_kotlin_source_returns_ok() {
    let result = extract_calls(Lang::Kotlin, KOTLIN_SOURCE);
    assert!(
        result.is_ok(),
        "S-002a: extract_calls must return Ok on Kotlin source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_references_on_kotlin_source_returns_ok() {
    let result = extract_references(Lang::Kotlin, KOTLIN_SOURCE);
    assert!(
        result.is_ok(),
        "S-002a: extract_references must return Ok on Kotlin source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_extends_on_kotlin_source_returns_ok() {
    let result = extract_extends(Lang::Kotlin, KOTLIN_SOURCE);
    assert!(
        result.is_ok(),
        "S-002a: extract_extends must return Ok on Kotlin source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_imports_on_kotlin_source_returns_ok() {
    let result = extract_imports(Lang::Kotlin, KOTLIN_SOURCE);
    assert!(
        result.is_ok(),
        "S-002a: extract_imports must return Ok on Kotlin source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_calls_on_empty_kotlin_source_returns_ok_empty() {
    // Edge: empty input — must still parse + return empty Vec, not error.
    let result = extract_calls(Lang::Kotlin, b"");
    assert!(
        result.is_ok(),
        "edge: empty Kotlin source must parse cleanly, got: {:?}",
        result.err()
    );
    assert!(result.unwrap().is_empty(), "empty source → no calls");
}
