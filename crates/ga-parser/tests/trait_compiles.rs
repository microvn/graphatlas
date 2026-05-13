//! Pin the trait shape: every registered lang implements all `LanguageSpec`
//! methods; trait-object dispatch works.
//!
//! (Expanded in extract_symbols.rs with real-source tests.)

use ga_core::Lang;
use ga_parser::{LanguageSpec, ParserPool};

#[test]
fn every_registered_lang_answers_lang_query() {
    let pool = ParserPool::new();
    for lang in [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
    ] {
        let spec: &dyn LanguageSpec = pool.spec_for(lang).expect("registered");
        assert_eq!(spec.lang(), lang);
    }
}

#[test]
fn symbol_node_predicate_is_lang_specific() {
    let pool = ParserPool::new();
    let py = pool.spec_for(Lang::Python).unwrap();
    let rs = pool.spec_for(Lang::Rust).unwrap();

    // Python-shaped node kinds.
    assert!(py.is_symbol_node("function_definition"));
    assert!(py.is_symbol_node("class_definition"));
    assert!(!py.is_symbol_node("function_item"));

    // Rust-shaped node kinds.
    assert!(rs.is_symbol_node("function_item"));
    assert!(rs.is_symbol_node("struct_item"));
    assert!(!rs.is_symbol_node("function_definition"));
}

#[test]
fn empty_pool_constructs_from_default() {
    let _pool = ParserPool::default();
}
