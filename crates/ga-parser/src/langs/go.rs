//! Go `LanguageSpec`. Grammar: `tree-sitter-go` 0.23.

use crate::references::{is_clean_ident, is_type_ident, last_named_child};
use crate::{LangFamily, LanguageSpec, ParsedReference, RefEmitter, RefKind};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct GoLang;

const SYMBOLS: &[&str] = &["function_declaration", "method_declaration", "type_spec"];
const IMPORTS: &[&str] = &["import_declaration", "import_spec"];
const CALLS: &[&str] = &["call_expression"];
// Go doesn't have class-style inheritance — interface satisfaction is implicit.
const EXTENDS: &[&str] = &[];

// S-005a D3 — `keyed_element` ref emitter, migrated from references.rs.
// Was hardcoded as `if matches!(lang, Lang::Go) { emit_go_keyed_element(...) }`
// inside the engine; now lives here and is registered via `ref_emitters()`.
//
// 2026-04-28 — added `type_identifier` emitter for type-position uses
// (var x Foo, Foo{...}, []Foo, func(Foo) etc.). Closes the dead_code FP
// gap where ga's indexer missed Go type-position references that
// Hd-ast's raw extraction picked up. See M3 dead_code audit.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[
    ("keyed_element", emit_keyed_element_struct_field),
    ("type_identifier", emit_type_position),
];

impl LanguageSpec for GoLang {
    fn lang(&self) -> Lang {
        Lang::Go
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn symbol_node_kinds(&self) -> &'static [&'static str] {
        SYMBOLS
    }
    fn import_node_kinds(&self) -> &'static [&'static str] {
        IMPORTS
    }
    fn call_node_kinds(&self) -> &'static [&'static str] {
        CALLS
    }
    fn extends_node_kinds(&self) -> &'static [&'static str] {
        EXTENDS
    }

    // extract_bases default-impl (empty) — Go has no class-style extends.

    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        crate::langs::shared::extract_import_path_default(node, source)
    }

    fn family(&self) -> LangFamily {
        LangFamily::SystemsGo
    }

    /// Gap 5 — Go test convention: `func TestX(t *testing.T)`.
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        let kind = node.kind();
        if kind != "function_declaration" && kind != "method_declaration" {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source) {
                // Go test convention — name = `Test` followed by uppercase
                // letter (`TestAdd`). Bare `Test` ambiguous; keep strict.
                if name.len() > 4
                    && name.starts_with("Test")
                    && name
                        .chars()
                        .nth(4)
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                {
                    attrs.push(crate::SymbolAttribute::TestMarker);
                }
                // Benchmarks too.
                if name.len() > 9
                    && name.starts_with("Benchmark")
                    && name
                        .chars()
                        .nth(9)
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                {
                    attrs.push(crate::SymbolAttribute::TestMarker);
                }
            }
        }
        attrs
    }

    /// PR5c2b — Go params: grouped-decl awareness. `func H(p, q string)` is
    /// a single `parameter_declaration` with 2 idents sharing a type. Walk
    /// `parameter_list` → for each `parameter_declaration`, collect each
    /// identifier as a separate ParsedParam with the shared type.
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        let kind = node.kind();
        if kind != "function_declaration" && kind != "method_declaration" {
            return Vec::new();
        }
        let plist = match node.child_by_field_name("parameters") {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut pc = plist.walk();
        for pdecl in plist.named_children(&mut pc) {
            // `parameter_declaration` (named) or `variadic_parameter_declaration`
            let ty = pdecl
                .child_by_field_name("type")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let mut dc = pdecl.walk();
            let mut idents: Vec<String> = pdecl
                .named_children(&mut dc)
                .filter(|c| c.kind() == "identifier")
                .filter_map(|n| n.utf8_text(source).ok())
                .map(|s| s.trim().to_string())
                .collect();
            if idents.is_empty() {
                idents.push(String::new()); // anonymous param (just type)
            }
            for name in idents {
                out.push(crate::ParsedParam {
                    name,
                    type_: ty.clone(),
                    default_value: String::new(),
                });
            }
        }
        out
    }

    /// PR5b — Go `function_declaration.result` / `method_declaration.result`.
    /// Multi-return `(int, error)` surfaces as a `parameter_list`; raw text
    /// is acceptable since UC consumers display it as-is.
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        let kind = node.kind();
        if kind != "function_declaration" && kind != "method_declaration" {
            return None;
        }
        crate::langs::shared::extract_return_type_by_field(node, source, "result")
    }

    /// PR5a — Go grouped params (`func H(p, q string)`) are a single
    /// `parameter_declaration` node carrying 2 identifiers. Default heuristic
    /// counts parameter_declaration children = 1; actual arity = 2. Override
    /// counts identifiers across each parameter_declaration.
    fn extract_arity(&self, node: &Node<'_>, _source: &[u8]) -> Option<i64> {
        let kind = node.kind();
        if !kind.contains("function") && !kind.contains("method") {
            return None;
        }
        // Find the `parameter_list` child (Go's param container).
        let mut cursor = node.walk();
        let plist = node
            .children(&mut cursor)
            .find(|c| c.kind() == "parameter_list")?;
        let mut total = 0i64;
        let mut pc = plist.walk();
        for pdecl in plist.named_children(&mut pc) {
            // Each `parameter_declaration` has zero+ identifier children
            // (`p, q` → 2 idents; `x` → 1 ident; variadic still 1 ident).
            // `variadic_parameter_declaration` → 1 ident.
            let mut id_count = 0i64;
            let mut dc = pdecl.walk();
            for sub in pdecl.named_children(&mut dc) {
                if sub.kind() == "identifier" {
                    id_count += 1;
                }
            }
            // Anonymous param (just type, e.g. `func _(int)`) → count 1.
            total += id_count.max(1);
        }
        Some(total)
    }

    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }
}

/// infra:S-001 AS-001 — Go `keyed_element` inside struct composite literal.
///
/// tree-sitter-go grammar emits `keyed_element` nodes with ordered children:
/// `[field_identifier, ":", value_expression]`. When the value is a bare
/// `identifier`, it's a candidate function reference (e.g.
/// `Handler{OnClick: handleClick}`). Filter via stopwords (rejects `nil`)
/// and `is_clean_ident`.
///
/// Body verbatim from references.rs::emit_go_keyed_element pre-D3.
fn emit_keyed_element_struct_field(
    keyed: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    // tree-sitter-go 0.23 wraps both key and value in `literal_element`
    // nodes. Last named child = value-side `literal_element` → its first
    // named child is the actual `identifier` (or other expression).
    let Some(value_wrapper) = last_named_child(keyed) else {
        return;
    };
    let value = if value_wrapper.kind() == "literal_element" {
        let Some(inner) = value_wrapper.named_child(0) else {
            return;
        };
        inner
    } else {
        value_wrapper
    };
    if value.kind() != "identifier" {
        return;
    }
    let Ok(name) = value.utf8_text(source) else {
        return;
    };
    if name == "nil" || !is_clean_ident(name) {
        return;
    }
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name: name.to_string(),
        ref_site_line: (value.start_position().row as u32) + 1,
        ref_kind: RefKind::StructFieldFn,
    });
}

/// 2026-04-28 — Go `type_identifier` emitter.
///
/// tree-sitter-go uses a dedicated `type_identifier` node for type names
/// (vs `identifier` for value names), so emitting on this node kind
/// directly captures all type-position references: `var x Foo`,
/// `Foo{...}`, `&Foo`, `[]Foo`, `func(Foo)`, `map[K]Foo`, etc.
///
/// The `type_spec` node (own-definition) is handled at the symbol layer,
/// not here — but children of `type_spec` may include the SAME name as a
/// recursive reference (e.g. `type T struct { next *T }`). We skip when
/// the parent is a `type_spec` and the parent's name child equals the
/// current node — keeps the noise out of the dead-code precision metric.
///
/// Stopwords: Go primitives + standard generic typenames. Without this
/// filter, every `int`/`string`/`bool` use would emit a TypePosition
/// edge → universe explosion + meaningless graph noise.
fn emit_type_position(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let Ok(name) = node.utf8_text(source) else {
        return;
    };
    if !is_type_ident(name) || is_go_primitive(name) {
        return;
    }
    // Skip if this `type_identifier` is the NAME slot of its own type_spec
    // (i.e. the definition site). Walk parent chain: type_spec's first
    // named child is the name node itself.
    if let Some(parent) = node.parent() {
        if parent.kind() == "type_spec" {
            if let Some(name_child) = parent.named_child(0) {
                if name_child.id() == node.id() {
                    return;
                }
            }
        }
    }
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name: name.to_string(),
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::TypePosition,
    });
}

fn is_go_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "byte"
            | "rune"
            | "string"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "float32"
            | "float64"
            | "complex64"
            | "complex128"
            | "error"
            | "any"
            | "comparable"
    )
}
