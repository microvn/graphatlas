//! AS-010 literal: LanguageSpec exposes `*_node_kinds()` returning the full
//! list of AST node kinds this lang cares about. Predicates (`is_*_node`)
//! remain as convenience but must now be derived from these lists.

use ga_core::Lang;
use ga_parser::{LanguageSpec, ParserPool};

fn spec(lang: Lang) -> Box<dyn LanguageSpec> {
    match lang {
        Lang::Python => Box::new(ga_parser::langs::py::PythonLang),
        Lang::TypeScript => Box::new(ga_parser::langs::ts::TypeScriptLang),
        Lang::JavaScript => Box::new(ga_parser::langs::js::JavaScriptLang),
        Lang::Go => Box::new(ga_parser::langs::go::GoLang),
        Lang::Rust => Box::new(ga_parser::langs::rs::RustLang),
        // v1.1-M4 — Phase C langs (Java/Kotlin/CSharp/Ruby) have Lang variants
        // but no LanguageSpec impl yet; test arrays below iterate v1 langs only.
        // AS-017 covers the "registered-but-no-spec" path via ParserPool.spec_for.
        Lang::Java | Lang::Kotlin | Lang::CSharp | Lang::Ruby | Lang::Php => {
            unreachable!("trait_api test fixture: {lang:?} not wired in this test (covered by per-lang test suites)")
        }
    }
}

#[test]
fn python_symbol_kinds_checklist() {
    let s = spec(Lang::Python);
    let kinds = s.symbol_node_kinds();
    assert!(kinds.contains(&"function_definition"), "{kinds:?}");
    assert!(kinds.contains(&"class_definition"), "{kinds:?}");
}

#[test]
fn rust_symbol_kinds_checklist() {
    let s = spec(Lang::Rust);
    let kinds = s.symbol_node_kinds();
    for k in [
        "function_item",
        "struct_item",
        "enum_item",
        "trait_item",
        "impl_item",
    ] {
        assert!(kinds.contains(&k), "rust missing {k}: {kinds:?}");
    }
}

#[test]
fn typescript_call_kinds_checklist() {
    let s = spec(Lang::TypeScript);
    let kinds = s.call_node_kinds();
    for k in ["call_expression", "new_expression"] {
        assert!(kinds.contains(&k), "{kinds:?}");
    }
}

#[test]
fn go_extends_kinds_empty() {
    let s = spec(Lang::Go);
    assert!(s.extends_node_kinds().is_empty());
}

#[test]
fn import_kinds_per_lang() {
    assert!(spec(Lang::Python)
        .import_node_kinds()
        .contains(&"import_statement"));
    assert!(spec(Lang::Python)
        .import_node_kinds()
        .contains(&"import_from_statement"));
    assert!(spec(Lang::Go)
        .import_node_kinds()
        .contains(&"import_declaration"));
    assert!(spec(Lang::Rust)
        .import_node_kinds()
        .contains(&"use_declaration"));
}

#[test]
fn is_predicates_derive_from_node_kinds_lists() {
    // Contract: is_symbol_node(k) ⟺ symbol_node_kinds().contains(&k).
    // This lets both APIs coexist without drift.
    let pool = ParserPool::new();
    for lang in [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
    ] {
        let s = pool.spec_for(lang).unwrap();
        for &k in s.symbol_node_kinds() {
            assert!(s.is_symbol_node(k), "{lang:?}: is_symbol_node({k}) = false");
        }
        for &k in s.import_node_kinds() {
            assert!(s.is_import_node(k), "{lang:?}: is_import_node({k}) = false");
        }
        for &k in s.call_node_kinds() {
            assert!(s.is_call_node(k), "{lang:?}: is_call_node({k}) = false");
        }
        for &k in s.extends_node_kinds() {
            assert!(
                s.is_extends_node(k),
                "{lang:?}: is_extends_node({k}) = false"
            );
        }
    }
}
