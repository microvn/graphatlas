//! v1.1-M4 (S-005a D4) — verify Phase A per-lang call-name kinds moved
//! out of `calls.rs` engine into per-lang `callee_extractors()` tables.
//!
//! These tests assert REGISTRATION. Behavior preservation lives in
//! `tests/extract_calls.rs`, `tests/extract_calls_arrow.rs`, and
//! `tests/extract_calls_phase_a_kinds.rs`.

use ga_core::Lang;
use ga_parser::ParserPool;

fn registered_kinds(lang: Lang) -> Vec<&'static str> {
    let pool = ParserPool::new();
    let spec = pool.spec_for(lang).unwrap();
    spec.callee_extractors().iter().map(|(k, _)| *k).collect()
}

#[test]
fn python_registers_decorator_callee_extractor() {
    let kinds = registered_kinds(Lang::Python);
    assert!(
        kinds.contains(&"decorator"),
        "Python must register decorator callee extractor (was hardcoded in calls.rs::extract_call_name pre-D4); got {kinds:?}"
    );
}

#[test]
fn rust_registers_macro_invocation_callee_extractor() {
    let kinds = registered_kinds(Lang::Rust);
    assert!(
        kinds.contains(&"macro_invocation"),
        "Rust must register macro_invocation callee extractor; got {kinds:?}"
    );
}

#[test]
fn typescript_registers_new_expression_callee_extractor() {
    let kinds = registered_kinds(Lang::TypeScript);
    assert!(
        kinds.contains(&"new_expression"),
        "TypeScript must register new_expression callee extractor; got {kinds:?}"
    );
}

#[test]
fn javascript_registers_new_expression_and_jsx_callee_extractors() {
    let kinds = registered_kinds(Lang::JavaScript);
    for required in &[
        "new_expression",
        "jsx_self_closing_element",
        "jsx_opening_element",
    ] {
        assert!(
            kinds.contains(required),
            "JavaScript must register `{required}` callee extractor; got {kinds:?}"
        );
    }
}

#[test]
fn go_does_not_register_lang_specific_callee_extractor() {
    // Go relies entirely on the engine's standard call_expression handling
    // (function field → identifier or selector_expression). No special kinds.
    assert!(registered_kinds(Lang::Go).is_empty());
}
