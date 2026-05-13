//! v1.1-M4 (S-005a D2) — `LanguageSpec` trait extension surface.
//!
//! Covers the fn-pointer table slots + family/convention/attribute hooks.
//! Engine refactor that consumes these slots ships in D3/D4 (separate tests).
//!
//! Design rationale (mf-voices Codex review): per-lang declares DATA
//! (handler tables, family enum, convention patterns) — engine reads tables.
//! Adding a new language = register in a table, no engine churn. AS-015
//! "no changes to shared extraction engine when adding a lang" thus enforced
//! by `engine_no_lang_match.rs` regression guard (D6).

use ga_core::Lang;
use ga_parser::{
    CalleeExtractor, ConventionPair, LangFamily, ParserPool, RefEmitter, SymbolAttribute,
};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Type-aliases compile + accept real handler shapes
// ---------------------------------------------------------------------------

fn sample_callee(_node: &Node<'_>, _source: &[u8]) -> Option<String> {
    Some("sample".to_string())
}

fn sample_ref_emitter(
    _node: &Node<'_>,
    _source: &[u8],
    _enclosing: &Option<String>,
    _out: &mut Vec<ga_parser::ParsedReference>,
) {
}

#[test]
fn callee_extractor_alias_accepts_fn_pointer() {
    let table: &[(&str, CalleeExtractor)] = &[("call_expression", sample_callee)];
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].0, "call_expression");
}

#[test]
fn ref_emitter_alias_accepts_fn_pointer() {
    let table: &[(&str, RefEmitter)] = &[("keyed_element", sample_ref_emitter)];
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].0, "keyed_element");
}

// ---------------------------------------------------------------------------
// Trait methods exist with default impls — calling them on existing langs
// must not panic and must return the documented defaults.
// ---------------------------------------------------------------------------

fn all_v1_langs() -> [Lang; 5] {
    [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
    ]
}

#[test]
fn callee_extractors_post_d4_migration_state() {
    // D2 introduced the slot. D4 migrated calls.rs per-kind branches
    // (decorator → Python, new_expression → TS+JS, jsx → JS, macro_invocation
    // → Rust) into per-lang tables. Go relies on engine's standard handling.
    let pool = ParserPool::new();
    let expected_kinds: &[(Lang, &[&str])] = &[
        (Lang::Python, &["decorator"]),
        (Lang::Go, &[]),
        (Lang::Rust, &["macro_invocation"]),
        (Lang::TypeScript, &["new_expression"]),
        (
            Lang::JavaScript,
            &[
                "new_expression",
                "jsx_self_closing_element",
                "jsx_opening_element",
            ],
        ),
    ];
    for &(lang, expected) in expected_kinds {
        let spec = pool.spec_for(lang).unwrap();
        let kinds: Vec<&str> = spec.callee_extractors().iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, expected, "{lang:?}");
    }
}

#[test]
fn ref_emitters_post_d3_migration_state() {
    // D2 introduced the slot. D3 migrated references.rs Phase A special cases
    // (Go keyed_element, Rust call_expression arg, JS/TS shorthand) into
    // per-lang tables. After D3: Go/Rust/TS/JS register; Python relies on
    // engine's structural pair/array emitters.
    let pool = ParserPool::new();
    // 2026-04-28 — Go/Rust/TS gained `type_identifier` emitter for
    // type-position REFERENCES (closes M3 dead_code FP gap; see
    // tests/extract_references_type_position.rs). JS pure (.js) intentionally
    // skipped — no `type_identifier` node kind in tree-sitter-javascript.
    let expected_kinds: &[(Lang, &[&str])] = &[
        (Lang::Python, &[]),
        (Lang::Go, &["keyed_element", "type_identifier"]),
        (Lang::Rust, &["call_expression", "type_identifier"]),
        (
            Lang::TypeScript,
            &["shorthand_property_identifier", "type_identifier"],
        ),
        (Lang::JavaScript, &["shorthand_property_identifier"]),
    ];
    for &(lang, expected) in expected_kinds {
        let spec = pool.spec_for(lang).unwrap();
        let kinds: Vec<&str> = spec.ref_emitters().iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, expected, "{lang:?}");
    }
}

#[test]
fn convention_pairs_default_empty() {
    // Convention pairs land per-lang as part of S-001..S-004 work
    // (Rails/Django/Spring/NestJS conventions). D2 just defines the slot.
    let pool = ParserPool::new();
    for lang in all_v1_langs() {
        let spec = pool.spec_for(lang).unwrap();
        assert!(spec.convention_pairs().is_empty(), "{lang:?}");
    }
}

#[test]
fn extract_attributes_default_empty_for_existing_langs() {
    // Default impl returns empty vec. Java @Service, Kotlin suspend, C# partial
    // override this when their per-lang impls land.
    let pool = ParserPool::new();
    let mut parser = tree_sitter::Parser::new();
    for lang in all_v1_langs() {
        let spec = pool.spec_for(lang).unwrap();
        parser.set_language(&spec.tree_sitter_lang()).unwrap();
        let tree = parser.parse(b"// dummy", None).unwrap();
        let attrs = spec.extract_attributes(&tree.root_node(), b"// dummy");
        assert!(attrs.is_empty(), "{lang:?}");
    }
}

// ---------------------------------------------------------------------------
// LangFamily — classification of existing langs
// ---------------------------------------------------------------------------

#[test]
fn family_classifies_v1_langs() {
    let pool = ParserPool::new();
    let cases: &[(Lang, LangFamily)] = &[
        (Lang::Python, LangFamily::DynamicScripting),
        (Lang::TypeScript, LangFamily::JsLike),
        (Lang::JavaScript, LangFamily::JsLike),
        (Lang::Go, LangFamily::SystemsGo),
        (Lang::Rust, LangFamily::SystemsRust),
    ];
    for &(lang, expected) in cases {
        let spec = pool.spec_for(lang).unwrap();
        assert_eq!(spec.family(), expected, "{lang:?}");
    }
}

#[test]
fn lang_family_enum_has_v1_1_variants() {
    // Compile-check: variants needed by Phase C langs exist on the enum.
    // Java/Kotlin/CSharp share StaticManaged (typed dispatch + annotations).
    // Ruby joins DynamicScripting (already exists).
    let _ = LangFamily::StaticManaged;
    let _ = LangFamily::JsLike;
    let _ = LangFamily::DynamicScripting;
    let _ = LangFamily::SystemsRust;
    let _ = LangFamily::SystemsGo;
    let _ = LangFamily::SystemsCfamily; // v2+ slot for C/C++/Zig
    let _ = LangFamily::Other;
}

// ---------------------------------------------------------------------------
// ConventionPair + SymbolAttribute shape
// ---------------------------------------------------------------------------

#[test]
fn convention_pair_struct_shape() {
    let p = ConventionPair {
        src_pattern: "app/models/{name}.rb",
        test_pattern: "spec/models/{name}_spec.rb",
        confidence: 0.9,
    };
    assert_eq!(p.src_pattern, "app/models/{name}.rb");
    assert_eq!(p.test_pattern, "spec/models/{name}_spec.rb");
    assert!((p.confidence - 0.9).abs() < f32::EPSILON);
}

#[test]
fn symbol_attribute_variants_cover_phase_c_needs() {
    // Variants Phase C will use:
    //   Kotlin S-002 → Suspend, ExtendedReceiver
    //   C# S-003 → Partial
    //   Java S-001 → Annotation (for @Service / @Autowired)
    //   Ruby S-004 → Decorator (for `define_method` polymorphic confidence)
    let _ = SymbolAttribute::Async;
    let _ = SymbolAttribute::Suspend;
    let _ = SymbolAttribute::Partial;
    let _ = SymbolAttribute::Static;
    let _ = SymbolAttribute::Override;
    let _ = SymbolAttribute::Const;
    let _ = SymbolAttribute::Annotation("Service".to_string());
    let _ = SymbolAttribute::Decorator {
        name: "define_method".to_string(),
        args: String::new(),
    };
    let _ = SymbolAttribute::ExtendedReceiver("String".to_string());
}
