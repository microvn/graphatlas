//! v1.1-M4 S-003a — CSharp LanguageSpec skeleton contract.
//!
//! Pin the **registration-level** invariants for `Lang::CSharp` independent
//! of per-AS extraction logic (which lands in S-003b/S-003c). These tests
//! flip the inverse contract previously held by `language_spec_unknown.rs`
//! (CSharp was "spec not registered") and replace it with positive
//! assertions:
//!
//!   - `ParserPool::new()` registers a `LanguageSpec` for `Lang::CSharp`
//!   - all four AST node-kind lists are non-empty + cover the baseline
//!     nodes the per-AS tests in S-003b/c will depend on
//!   - the four public extractors return `Ok` on a C# source — concrete
//!     shape of extracted records belongs to S-003b/c per-AS tests
//!   - `family()` reports `LangFamily::StaticManaged` (Java/Kotlin/Scala/C#)
//!
//! The "no-spec → typed Err" path is still covered for AS-017 — the probe
//! moves to `Lang::Ruby` (still unregistered until S-004 lands), see
//! `language_spec_unknown.rs`.

use ga_core::Lang;
use ga_parser::{
    extract_calls, extract_extends, extract_imports, extract_references, LangFamily, ParserPool,
};

const CSHARP_SOURCE: &[u8] = b"\
using System;\n\
namespace App {\n\
    public class UserService {\n\
        public User GetUser() { return new User(); }\n\
    }\n\
}\n";

#[test]
fn registers_csharplang_in_pool() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::CSharp);
    assert!(
        spec.is_some(),
        "S-003a: ParserPool::new() must register a LanguageSpec for Lang::CSharp"
    );
    assert_eq!(spec.unwrap().lang(), Lang::CSharp);
}

#[test]
fn csharp_family_is_static_managed() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::CSharp).expect("CSharp registered");
    assert_eq!(
        spec.family(),
        LangFamily::StaticManaged,
        "S-003a: CSharp family must be StaticManaged (groups JVM+.NET typed-managed langs)"
    );
}

#[test]
fn csharp_node_kind_lists_non_empty() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::CSharp).expect("CSharp registered");
    assert!(!spec.symbol_node_kinds().is_empty(), "symbol_node_kinds");
    assert!(!spec.import_node_kinds().is_empty(), "import_node_kinds");
    assert!(!spec.call_node_kinds().is_empty(), "call_node_kinds");
    assert!(!spec.extends_node_kinds().is_empty(), "extends_node_kinds");
}

#[test]
fn csharp_node_kinds_include_baseline_set() {
    // Sanity floor: the spec promises C# symbols include classes +
    // interfaces + enums + records + structs + delegates + methods, imports
    // cover `using_directive`, calls cover invocation_expression +
    // object_creation_expression. Stronger AS-016 dynamic-drift coverage
    // lives in grammar_drift.rs.
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::CSharp).expect("CSharp registered");
    for required in &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "struct_declaration",
        "record_declaration",
        "delegate_declaration",
        "method_declaration",
        "constructor_declaration",
    ] {
        assert!(
            spec.symbol_node_kinds().contains(required),
            "symbol_node_kinds must include `{required}`"
        );
    }
    assert!(
        spec.import_node_kinds().contains(&"using_directive"),
        "import_node_kinds must include `using_directive` (C# grammar uses single using_directive for plain/static/alias forms)"
    );
    for required in &["invocation_expression", "object_creation_expression"] {
        assert!(
            spec.call_node_kinds().contains(required),
            "call_node_kinds must include `{required}`"
        );
    }
}

#[test]
fn extract_calls_on_csharp_source_returns_ok() {
    let result = extract_calls(Lang::CSharp, CSHARP_SOURCE);
    assert!(
        result.is_ok(),
        "S-003a: extract_calls must return Ok on C# source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_references_on_csharp_source_returns_ok() {
    let result = extract_references(Lang::CSharp, CSHARP_SOURCE);
    assert!(
        result.is_ok(),
        "S-003a: extract_references must return Ok on C# source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_extends_on_csharp_source_returns_ok() {
    let result = extract_extends(Lang::CSharp, CSHARP_SOURCE);
    assert!(
        result.is_ok(),
        "S-003a: extract_extends must return Ok on C# source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_imports_on_csharp_source_returns_ok() {
    let result = extract_imports(Lang::CSharp, CSHARP_SOURCE);
    assert!(
        result.is_ok(),
        "S-003a: extract_imports must return Ok on C# source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_calls_on_empty_csharp_source_returns_ok_empty() {
    // Edge: empty input — must still parse + return empty Vec, not error.
    let result = extract_calls(Lang::CSharp, b"");
    assert!(
        result.is_ok(),
        "edge: empty C# source must parse cleanly, got: {:?}",
        result.err()
    );
    assert!(result.unwrap().is_empty(), "empty source → no calls");
}
