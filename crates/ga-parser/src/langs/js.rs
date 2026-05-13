//! JavaScript `LanguageSpec`. Grammar: `tree-sitter-javascript` 0.23.

use crate::{CalleeExtractor, LangFamily, LanguageSpec, RefEmitter};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct JavaScriptLang;

// S-005a D3 — shorthand-property ref emitter migrated from references.rs,
// shared with TypeScript via langs/shared.rs.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[(
    "shorthand_property_identifier",
    crate::langs::shared::emit_shorthand_property_ref,
)];

// S-005a D4 — `new_expression` shared with TS, plus JSX uppercase callee
// (JS-only — TS pure (.ts) doesn't load JSX grammar).
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[
    (
        "new_expression",
        crate::langs::shared::extract_new_expression_callee,
    ),
    (
        "jsx_self_closing_element",
        crate::langs::shared::extract_jsx_element_callee,
    ),
    (
        "jsx_opening_element",
        crate::langs::shared::extract_jsx_element_callee,
    ),
];

const SYMBOLS: &[&str] = &[
    "function_declaration",
    "class_declaration",
    "method_definition",
    "arrow_function",
];
const IMPORTS: &[&str] = &["import_statement"];
const CALLS: &[&str] = &[
    "call_expression",
    "new_expression",
    "jsx_self_closing_element",
    "jsx_opening_element",
];
const EXTENDS: &[&str] = &["class_declaration", "class"];

impl LanguageSpec for JavaScriptLang {
    fn lang(&self) -> Lang {
        Lang::JavaScript
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_javascript::LANGUAGE.into()
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

    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        crate::langs::ts::ts_js_extract_bases(node, source)
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

    /// PR5c2b — JS modifiers: `async` keyword + parent `export_statement`.
    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_declaration" | "method_definition" | "arrow_function" | "function"
        ) {
            return Vec::new();
        }
        let mut mods = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "async" | "static") {
                if let Ok(t) = child.utf8_text(source) {
                    mods.push(t.trim().to_string());
                }
            }
        }
        if let Some(parent) = node.parent() {
            if parent.kind() == "export_statement" {
                mods.push("export".to_string());
            }
        }
        mods
    }

    /// PR5c2b — JS params: `parameters` field (formal_parameters node).
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        let kind = node.kind();
        if !matches!(
            kind,
            "function_declaration" | "method_definition" | "arrow_function" | "function"
        ) {
            return Vec::new();
        }
        crate::langs::shared::extract_params_by_container(node, source, "parameters")
    }
}
