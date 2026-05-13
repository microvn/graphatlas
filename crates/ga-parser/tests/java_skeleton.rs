//! v1.1-M4 S-001a — Java LanguageSpec skeleton contract.
//!
//! Pin the **registration-level** invariants for `Lang::Java` independent
//! of per-AS extraction logic (which lands in S-001b/S-001c). These tests
//! flip the inverse contract held by `language_spec_unknown.rs` (Java was
//! "spec not registered") and replace it with positive assertions:
//!
//!   - `ParserPool::new()` registers a `LanguageSpec` for `Lang::Java`
//!   - all four AST node-kind lists are non-empty
//!   - the four public extractors (`extract_calls/references/extends/
//!     imports`) return `Ok` on a Java source — concrete shape of
//!     extracted records belongs to S-001b/c per-AS tests
//!   - `family()` reports `LangFamily::StaticManaged` (Java/Kotlin/Scala/C#)
//!
//! The "no-spec → typed Err" path is still covered for AS-017 — the probe
//! lang rotates as Phase C stories ship: Kotlin in S-001a → CSharp in S-002a
//! (post-Kotlin-impl) → Ruby in S-003a (post-CSharp). After S-003a, Ruby
//! is the only remaining unregistered Phase C lang.

use ga_core::Lang;
use ga_parser::{
    extract_calls, extract_extends, extract_imports, extract_references, LangFamily, ParserPool,
};

const JAVA_SOURCE: &[u8] = b"\
package com.example;\n\
import com.example.util.Helper;\n\
public class UserService {\n\
    public User getUser() { return new User(); }\n\
}\n";

#[test]
fn registers_javalang_in_pool() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Java);
    assert!(
        spec.is_some(),
        "S-001a: ParserPool::new() must register a LanguageSpec for Lang::Java"
    );
    assert_eq!(spec.unwrap().lang(), Lang::Java);
}

#[test]
fn java_family_is_static_managed() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Java).expect("Java registered");
    assert_eq!(
        spec.family(),
        LangFamily::StaticManaged,
        "S-001a: Java family must be StaticManaged (groups JVM+.NET typed-managed langs)"
    );
}

#[test]
fn java_node_kind_lists_non_empty() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Java).expect("Java registered");
    assert!(!spec.symbol_node_kinds().is_empty(), "symbol_node_kinds");
    assert!(!spec.import_node_kinds().is_empty(), "import_node_kinds");
    assert!(!spec.call_node_kinds().is_empty(), "call_node_kinds");
    assert!(!spec.extends_node_kinds().is_empty(), "extends_node_kinds");
}

#[test]
fn java_node_kinds_include_baseline_set() {
    // Sanity floor: the spec promises Java symbols include classes +
    // interfaces + methods, imports cover import_declaration, calls cover
    // method_invocation. Stronger AS-016 dynamic-drift coverage lives in
    // grammar_drift.rs.
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Java).expect("Java registered");
    assert!(
        spec.symbol_node_kinds().contains(&"class_declaration"),
        "symbol_node_kinds must include `class_declaration`"
    );
    assert!(
        spec.symbol_node_kinds().contains(&"interface_declaration"),
        "symbol_node_kinds must include `interface_declaration`"
    );
    assert!(
        spec.symbol_node_kinds().contains(&"method_declaration"),
        "symbol_node_kinds must include `method_declaration`"
    );
    assert!(
        spec.import_node_kinds().contains(&"import_declaration"),
        "import_node_kinds must include `import_declaration`"
    );
    assert!(
        spec.call_node_kinds().contains(&"method_invocation"),
        "call_node_kinds must include `method_invocation`"
    );
}

#[test]
fn extract_calls_on_java_source_returns_ok() {
    let result = extract_calls(Lang::Java, JAVA_SOURCE);
    assert!(
        result.is_ok(),
        "S-001a: extract_calls must return Ok on Java source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_references_on_java_source_returns_ok() {
    let result = extract_references(Lang::Java, JAVA_SOURCE);
    assert!(
        result.is_ok(),
        "S-001a: extract_references must return Ok on Java source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_extends_on_java_source_returns_ok() {
    let result = extract_extends(Lang::Java, JAVA_SOURCE);
    assert!(
        result.is_ok(),
        "S-001a: extract_extends must return Ok on Java source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_imports_on_java_source_returns_ok() {
    let result = extract_imports(Lang::Java, JAVA_SOURCE);
    assert!(
        result.is_ok(),
        "S-001a: extract_imports must return Ok on Java source once spec is registered, got: {:?}",
        result.err()
    );
}

#[test]
fn extract_calls_on_empty_java_source_returns_ok_empty() {
    // Edge: empty input — must still parse + return empty Vec, not error.
    let result = extract_calls(Lang::Java, b"");
    assert!(
        result.is_ok(),
        "edge: empty Java source must parse cleanly, got: {:?}",
        result.err()
    );
    assert!(result.unwrap().is_empty(), "empty source → no calls");
}

// Note: prior to S-004a there was a `ruby_remains_unregistered_probe` test
// here pinning the AS-017 typed-Err contract. After S-004a Ruby is registered
// (last phase-C lang shipped), so that probe has no target. The AS-017
// completeness contract is now in `language_spec_unknown.rs::all_v1_plus_phase_c_langs_registered`
// and `parser_pool_registers_all_v1_plus_phase_c_langs` — every Lang variant
// has a registered spec.
