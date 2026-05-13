//! v1.1-M4 (S-005a D5) — ParsedSymbol shape extension.
//!
//! Renames `enclosing_class: Option<String>` → `enclosing: Option<EnclosingScope>`
//! (typed enum: Class/ExtendedType/Module/Namespace). Adds `attributes:
//! Vec<SymbolAttribute>` (default empty) + `confidence: f32` (default 1.0)
//! so Phase C lang stories (Kotlin extension fns, C# partial, Java
//! annotations, Ruby metaprog) have slots without churning struct shape
//! again.

use ga_core::Lang;
use ga_parser::{parse_source, EnclosingScope, ParsedSymbol};

// ---------------------------------------------------------------------------
// EnclosingScope enum — variant coverage
// ---------------------------------------------------------------------------

#[test]
fn enclosing_scope_has_v1_1_variants() {
    // Compile-check: variants Phase C will need.
    let _ = EnclosingScope::Class("Foo".to_string());
    let _ = EnclosingScope::ExtendedType("String".to_string()); // Kotlin
    let _ = EnclosingScope::Module("auth".to_string()); // Python/Ruby
    let _ = EnclosingScope::Namespace("App".to_string()); // C#
}

#[test]
fn enclosing_scope_as_str_returns_inner_name() {
    // into_symbol maps EnclosingScope → Symbol.enclosing (String).
    // Each variant's as_str() returns the inner name unchanged.
    assert_eq!(EnclosingScope::Class("Foo".to_string()).as_str(), "Foo");
    assert_eq!(
        EnclosingScope::ExtendedType("String".to_string()).as_str(),
        "String"
    );
    assert_eq!(EnclosingScope::Module("auth".to_string()).as_str(), "auth");
    assert_eq!(EnclosingScope::Namespace("App".to_string()).as_str(), "App");
}

// ---------------------------------------------------------------------------
// ParsedSymbol new fields with defaults
// ---------------------------------------------------------------------------

fn parse_one(lang: Lang, src: &[u8]) -> Vec<ParsedSymbol> {
    parse_source(lang, src).expect("parse_source failed")
}

#[test]
fn parsed_symbol_has_attributes_field_defaulting_empty() {
    let syms = parse_one(Lang::Python, b"def f(): pass\n");
    let f = syms.iter().find(|s| s.name == "f").unwrap();
    assert!(
        f.attributes.is_empty(),
        "default attributes must be empty; got {:?}",
        f.attributes
    );
}

#[test]
fn parsed_symbol_has_confidence_field_defaulting_to_one() {
    let syms = parse_one(Lang::Python, b"def f(): pass\n");
    let f = syms.iter().find(|s| s.name == "f").unwrap();
    assert!(
        (f.confidence - 1.0).abs() < f32::EPSILON,
        "default confidence must be 1.0; got {}",
        f.confidence
    );
}

// ---------------------------------------------------------------------------
// enclosing renamed + typed
// ---------------------------------------------------------------------------

#[test]
fn module_level_function_has_no_enclosing() {
    let syms = parse_one(Lang::Python, b"def top_level(): pass\n");
    let f = syms.iter().find(|s| s.name == "top_level").unwrap();
    assert!(f.enclosing.is_none());
}

#[test]
fn method_inside_python_class_has_class_enclosing() {
    let syms = parse_one(Lang::Python, b"class Foo:\n    def bar(self): pass\n");
    let bar = syms.iter().find(|s| s.name == "bar").unwrap();
    match &bar.enclosing {
        Some(EnclosingScope::Class(name)) => assert_eq!(name, "Foo"),
        other => panic!("expected EnclosingScope::Class(\"Foo\"), got {other:?}"),
    }
}

// NOTE on Rust `impl Foo { fn bar(&self) {} }` — pre-D5 parser does NOT
// propagate `Foo` as enclosing for `bar` because `impl_item` lacks a `name`
// field that `name_from_node` resolves; walker never sets new_enclosing.
// D5 preserves this limitation verbatim (no behavior change). When Rust
// extension/method enclosing is needed, a separate parser-semantics fix
// (likely tracking `impl_item` via type-field lookup) is required —
// outside S-005a scope.

// ---------------------------------------------------------------------------
// into_symbol — String mapping for backward compat with ga_core::Symbol
// ---------------------------------------------------------------------------

#[test]
fn into_symbol_maps_enclosing_scope_to_string() {
    let syms = parse_one(Lang::Python, b"class Foo:\n    def bar(self): pass\n");
    let bar = syms.iter().find(|s| s.name == "bar").unwrap().clone();
    let symbol = bar.into_symbol("test.py");
    // Symbol.enclosing remains Option<String> (ga_core API stable).
    assert_eq!(symbol.enclosing.as_deref(), Some("Foo"));
}
