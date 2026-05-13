//! TypeScript `LanguageSpec`. Grammar: `tree-sitter-typescript` 0.23.

use crate::references::is_type_ident;
use crate::{CalleeExtractor, LangFamily, LanguageSpec, ParsedReference, RefEmitter, RefKind};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct TypeScriptLang;

// S-005a D3 — shorthand-property ref emitter migrated from references.rs.
//
// 2026-04-28 — added `type_identifier` emitter for TS type-position uses
// (let x: Foo, function arg: Foo, Foo<G>, extends Foo). Closes the
// dead_code FP gap on TS fixtures (nest 0.775 FAIL).
const REF_EMITTERS: &[(&str, RefEmitter)] = &[
    (
        "shorthand_property_identifier",
        crate::langs::shared::emit_shorthand_property_ref,
    ),
    ("type_identifier", emit_type_position),
];

// S-005a D4 — `new_expression` callee extractor migrated from calls.rs.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[(
    "new_expression",
    crate::langs::shared::extract_new_expression_callee,
)];

const SYMBOLS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "method_definition",
    "arrow_function",
    "interface_declaration",
];
const IMPORTS: &[&str] = &["import_statement"];
// We load `LANGUAGE_TYPESCRIPT`, not `LANGUAGE_TSX`. Pure TypeScript has no
// JSX node kinds. `.tsx` support lands in v1.1 when we also pin tree-sitter-tsx.
const CALLS: &[&str] = &["call_expression", "new_expression"];
const EXTENDS: &[&str] = &["class_declaration", "class"];

impl LanguageSpec for TypeScriptLang {
    fn lang(&self) -> Lang {
        Lang::TypeScript
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
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

    /// rust-poc/src/main.rs:304-322 — walk `class_heritage` children, collect
    /// `identifier` / `type_identifier`.
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        ts_js_extract_bases(node, source)
    }

    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        crate::langs::shared::extract_import_path_default(node, source)
    }

    fn family(&self) -> LangFamily {
        LangFamily::JsLike
    }

    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }

    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }

    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        crate::imports::ts_js_imported_names(node, source)
    }

    fn extract_imported_aliases(&self, node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
        crate::imports::ts_js_imported_aliases(node, source)
    }

    fn extract_re_export(&self, node: &Node<'_>, source: &[u8]) -> Option<(String, Vec<String>)> {
        crate::imports::ts_js_extract_re_export(node, source)
    }

    /// v1.4 S-002 / AS-015..017 — TS-only: collect names imported with
    /// the `type` modifier. Three shapes:
    ///   1. Whole-statement `import type { Foo, Bar } from 'mod'`
    ///      → all names are type-only.
    ///   2. Per-name `import { Foo, type Bar } from 'mod'`
    ///      → only names with the `type` keyword on their import_specifier
    ///        are type-only.
    ///   3. Re-export `export type { Foo } from 'mod'` (AS-017)
    ///      → all names are type-only.
    /// For aliased forms (`import { X as Y }`) the LOCAL name (Y) is
    /// reported, mirroring extract_imported_names.
    fn extract_type_only_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        crate::imports::ts_js_type_only_names(node, source)
    }

    /// PR5c2b — TS modifiers: collect direct sibling keyword children
    /// (`async`) before the `name` field. Export sits on the wrapping
    /// `export_statement` (parent) — captured if present.
    /// v1.4 S-001a-prereq AS-020 — TypeScript 4.3+ `override` modifier on
    /// class methods. Tree-sitter-typescript exposes the modifier as an
    /// `override_modifier` token under `method_definition` /
    /// `method_signature`. Push SymbolAttribute::Override when present.
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        let kind = node.kind();
        if !matches!(kind, "method_definition" | "method_signature") {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            // tree-sitter-typescript exposes the override keyword either as
            // a typed `override_modifier` AST node OR as an anonymous
            // `override` keyword token (grammar drift across versions —
            // both shapes have been observed). Match both.
            if child.kind() == "override_modifier" {
                attrs.push(crate::SymbolAttribute::Override);
                continue;
            }
            if child.kind() == "override" {
                if let Ok(t) = child.utf8_text(source) {
                    if t.trim() == "override" {
                        attrs.push(crate::SymbolAttribute::Override);
                    }
                }
            }
        }
        attrs
    }

    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_declaration"
                | "method_definition"
                | "function_signature"
                | "method_signature"
                | "arrow_function"
                | "function"
        ) {
            return Vec::new();
        }
        let mut mods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if matches!(ck, "async" | "static" | "abstract" | "readonly") {
                if let Ok(t) = child.utf8_text(source) {
                    mods.push(t.trim().to_string());
                }
            }
            if matches!(
                ck,
                "accessibility_modifier" | "override_modifier" | "public" | "private" | "protected"
            ) {
                if let Ok(t) = child.utf8_text(source) {
                    mods.push(t.trim().to_string());
                }
            }
        }
        // Walk up: export_statement parent.
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                mods.push("export".to_string());
            }
        }
        mods
    }

    /// PR5c2b — TS params: container field `parameters`. Tree-sitter-typescript
    /// uses `required_parameter` / `optional_parameter` / `rest_parameter`
    /// children, each with `pattern` + optional `type` (type_annotation).
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_declaration"
                | "method_definition"
                | "function_signature"
                | "method_signature"
                | "arrow_function"
                | "function"
        ) {
            return Vec::new();
        }
        crate::langs::shared::extract_params_by_container(node, source, "parameters")
    }

    /// PR5b — TS `return_type` field. Grammar wraps in `type_annotation`
    /// (leading `:`); shared helper strips. Applies to function_declaration,
    /// method_definition, function_signature, arrow_function.
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_declaration"
                | "method_definition"
                | "function_signature"
                | "arrow_function"
                | "method_signature"
                | "function"
        ) {
            return None;
        }
        crate::langs::shared::extract_return_type_by_field(node, source, "return_type")
    }
}

pub(crate) fn ts_js_extract_bases(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "class_heritage" {
            continue;
        }
        let mut inner = child.walk();
        for heritage in child.children(&mut inner) {
            let mut deep = heritage.walk();
            for item in heritage.children(&mut deep) {
                if item.kind() == "identifier" || item.kind() == "type_identifier" {
                    if let Ok(text) = item.utf8_text(source) {
                        if text.len() > 1 {
                            bases.push(text.to_string());
                        }
                    }
                }
            }
        }
    }
    bases
}

/// 2026-04-28 — TS `type_identifier` emitter.
///
/// tree-sitter-typescript uses a dedicated `type_identifier` node for
/// type-position uses — `let x: Foo`, `function f(x: Foo)`, `Foo<G>`,
/// `extends Foo`, `implements Foo`. Value-position uses of the same
/// name go through `identifier`, so this emitter is type-only by
/// grammar.
///
/// Skip when this `type_identifier` is the NAME slot of its own
/// `(class|interface|type_alias)_declaration` — the definition site.
///
/// Stopwords: TS built-in types. Without filter, `string`/`number`/etc.
/// would explode the universe.
fn emit_type_position(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let Ok(name) = node.utf8_text(source) else {
        return;
    };
    if !is_type_ident(name) || is_ts_builtin(name) {
        return;
    }
    if let Some(parent) = node.parent() {
        let parent_kind = parent.kind();
        let is_def_parent = matches!(
            parent_kind,
            "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "enum_declaration"
        );
        if is_def_parent {
            if let Some(name_child) = parent.child_by_field_name("name") {
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

fn is_ts_builtin(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "number"
            | "boolean"
            | "void"
            | "any"
            | "unknown"
            | "never"
            | "object"
            | "undefined"
            | "null"
            | "bigint"
            | "symbol"
            | "Array"
            | "Promise"
            | "Map"
            | "Set"
            | "Record"
            | "Partial"
            | "Required"
            | "Readonly"
            | "Pick"
            | "Omit"
            | "Exclude"
            | "Extract"
            | "ReturnType"
            | "Awaited"
            | "Date"
            | "RegExp"
            | "Error"
            | "Function"
            | "Object"
    )
}
