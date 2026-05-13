//! Shared tree walker. Visits every node; per-lang predicates via `LanguageSpec`.
//! Ported from `rust-poc/src/main.rs:956-1162` (extract_from_tree) — simplified
//! to only emit symbols for S-004; imports, calls, extends land in follow-up
//! stories that need them.

use crate::{classify_kind, name_from_node, EnclosingScope, LanguageSpec, ParsedSymbol};
use tree_sitter::Node;

pub fn walk_tree(
    root: Node<'_>,
    source: &[u8],
    spec: &dyn LanguageSpec,
    enclosing_class: Option<&str>,
    out: &mut Vec<ParsedSymbol>,
) {
    walk_node(
        root,
        source,
        spec,
        enclosing_class.map(|s| EnclosingScope::Class(s.to_string())),
        out,
    );
}

fn walk_node(
    node: Node<'_>,
    source: &[u8],
    spec: &dyn LanguageSpec,
    enclosing: Option<EnclosingScope>,
    out: &mut Vec<ParsedSymbol>,
) {
    let kind = node.kind();
    let mut new_enclosing = enclosing.clone();

    // v1.1-M4 S-004c (AS-014) — synthetic-symbol hook for langs whose
    // metaprogramming defines methods via call expressions
    // (Ruby `define_method`, Python `setattr`, JS `Object.defineProperty`).
    // The walker invokes this on EVERY node; per-lang impls return Some only
    // for the patterns they recognize. Default = None for langs without
    // metaprogramming (Java/Kotlin/Go/Rust/etc.).
    //
    // Synthetic symbols carry `confidence < 1.0` per Tools-C11 — indexer
    // surfaces this to downstream consumers (ga_symbols meta.warning).
    if let Some(synthetic) = spec.extract_synthetic_symbol(&node, source) {
        out.push(ParsedSymbol {
            enclosing: synthetic.enclosing.or_else(|| enclosing.clone()),
            ..synthetic
        });
    }

    // PR4 / AS-005(a) — container-name fallback (Rust impl_item: type field
    // → enclosing). Runs even when name_from_node returns None so children
    // of `impl Foo` see Foo as Class enclosing. Pure enclosing-update; does
    // NOT push a Symbol row for the container.
    if let Some(container_name) = spec.container_name_fallback(&node, source) {
        new_enclosing = Some(EnclosingScope::Class(container_name));
    }

    if spec.is_symbol_node(kind) {
        if let Some(name) = name_from_node(&node, source) {
            let sym_kind = classify_kind(kind);
            // Track class-like names so nested methods record enclosing scope.
            // S-005a D5: variant `Class` covers all OOP container kinds
            // (struct/trait/interface/enum) — they share dispatch semantics
            // for "method belongs to type X". Per-lang impls override with
            // ExtendedType (Kotlin) / Module (Python module-level) / Namespace
            // (C#) when they ship.
            if matches!(
                sym_kind,
                ga_core::SymbolKind::Class
                    | ga_core::SymbolKind::Struct
                    | ga_core::SymbolKind::Trait
                    | ga_core::SymbolKind::Interface
                    | ga_core::SymbolKind::Enum
            ) {
                new_enclosing = Some(EnclosingScope::Class(name.clone()));
            }
            // S-002c — per-lang enclosing override (Kotlin extension fns:
            // `fun String.foo()` → EnclosingScope::ExtendedType("String")
            // regardless of containing class). Override applies to THIS
            // symbol only; children still see the inherited Class scope.
            let symbol_enclosing = spec
                .enclosing_for_symbol(&node, source)
                .or_else(|| enclosing.clone());
            out.push(ParsedSymbol {
                name,
                kind: sym_kind,
                line: (node.start_position().row as u32) + 1,
                line_end: (node.end_position().row as u32) + 1,
                enclosing: symbol_enclosing,
                attributes: spec.extract_attributes(&node, source),
                confidence: 1.0,
                arity: spec.extract_arity(&node, source),
                return_type: spec.extract_return_type(&node, source),
                modifiers: spec.extract_modifiers(&node, source),
                params: spec.extract_params(&node, source),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, source, spec, new_enclosing.clone(), out);
    }
}
