//! v1.1-M4 (S-005a D3) — verify Phase A per-lang reference cases moved out
//! of `references.rs` engine into per-lang `ref_emitters()` tables.
//!
//! These tests assert REGISTRATION (table contains the expected handler).
//! Behavior preservation (byte-identical output for existing fixtures) is
//! covered by the pre-existing tests in `tests/references.rs` — they are
//! the regression net for this migration.

use ga_core::Lang;
use ga_parser::ParserPool;

#[test]
fn go_registers_keyed_element_ref_emitter() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Go).unwrap();
    let kinds: Vec<&str> = spec.ref_emitters().iter().map(|(k, _)| *k).collect();
    assert!(
        kinds.contains(&"keyed_element"),
        "Go must register keyed_element ref emitter (was hardcoded in references.rs::walk via `if matches!(lang, Lang::Go)` pre-D3); got {kinds:?}"
    );
}

#[test]
fn rust_registers_call_expression_ref_emitter() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Rust).unwrap();
    let kinds: Vec<&str> = spec.ref_emitters().iter().map(|(k, _)| *k).collect();
    assert!(
        kinds.contains(&"call_expression"),
        "Rust must register call_expression ref emitter (fn-pointer arg, infra:S-001 AS-002); got {kinds:?}"
    );
}

#[test]
fn typescript_registers_shorthand_ref_emitter() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::TypeScript).unwrap();
    let kinds: Vec<&str> = spec.ref_emitters().iter().map(|(k, _)| *k).collect();
    assert!(
        kinds.contains(&"shorthand_property_identifier"),
        "TypeScript must register shorthand_property_identifier ref emitter; got {kinds:?}"
    );
}

#[test]
fn javascript_registers_shorthand_ref_emitter() {
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::JavaScript).unwrap();
    let kinds: Vec<&str> = spec.ref_emitters().iter().map(|(k, _)| *k).collect();
    assert!(
        kinds.contains(&"shorthand_property_identifier"),
        "JavaScript must register shorthand_property_identifier ref emitter; got {kinds:?}"
    );
}

#[test]
fn python_does_not_register_lang_specific_ref_emitter() {
    // Python has no lang-specific ref emitter (pair/list dict patterns are
    // handled by engine's structural emitters, not per-lang). This test
    // pins the contract: D3 only migrates Phase A additions, nothing else.
    let pool = ParserPool::new();
    let spec = pool.spec_for(Lang::Python).unwrap();
    assert!(
        spec.ref_emitters().is_empty(),
        "Python should rely on engine's structural pair/array emitters"
    );
}
