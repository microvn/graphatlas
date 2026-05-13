//! Python `LanguageSpec`. Grammar: `tree-sitter-python` 0.23 (pinned in
//! Cargo.lock per AS-010). Predicates ported from rust-poc/src/main.rs:244+.

use crate::{CalleeExtractor, LangFamily, LanguageSpec};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct PythonLang;

// AS-010 checklist: AST node kinds tree-sitter-python 0.23 emits for each
// category. Any grammar bump must update these lists or tests will catch it.
const SYMBOLS: &[&str] = &["function_definition", "class_definition"];
const IMPORTS: &[&str] = &["import_statement", "import_from_statement"];
const CALLS: &[&str] = &["call", "decorator"];
const EXTENDS: &[&str] = &["class_definition"];

// S-005a D4 — `decorator` callee extractor, migrated from calls.rs.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[("decorator", extract_decorator_callee)];

impl LanguageSpec for PythonLang {
    fn lang(&self) -> Lang {
        Lang::Python
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_python::LANGUAGE.into()
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

    /// rust-poc/src/main.rs:289-302 — iterate `superclasses` field, take the
    /// trailing identifier after any module prefix (`pkg.Base` → `Base`).
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();
        let Some(args) = node.child_by_field_name("superclasses") else {
            return bases;
        };
        let mut cursor = args.walk();
        for child in args.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "attribute" {
                if let Ok(text) = child.utf8_text(source) {
                    let name = text.rsplit('.').next().unwrap_or(text);
                    if name.len() > 1 {
                        bases.push(name.to_string());
                    }
                }
            }
        }
        bases
    }

    /// rust-poc/src/main.rs:382-417 shared path; Python import-from has
    /// `dotted_name` as the module.
    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        crate::langs::shared::extract_import_path_default(node, source)
    }

    fn family(&self) -> LangFamily {
        LangFamily::DynamicScripting
    }

    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }

    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        crate::imports::python_imported_names(node, source)
    }

    fn extract_imported_aliases(&self, node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
        crate::imports::python_imported_aliases(node, source)
    }

    /// PR8 — Python decorator extraction. tree-sitter-python wraps a
    /// decorated function in `decorated_definition` whose children are
    /// `decorator` nodes followed by the inner `function_definition`. The
    /// walker reaches the inner function_definition first; here we look up
    /// to the parent and harvest each decorator's callee name (via the
    /// existing `extract_decorator_callee` helper) into a
    /// `SymbolAttribute::Decorator(name)` entry. Indexer consumes these
    /// to emit DECORATES edges.
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        if node.kind() != "function_definition" {
            return Vec::new();
        }
        let mut out = Vec::new();
        // Gap 5 — Python pytest convention: `def test_*` is a test marker.
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source) {
                if name.starts_with("test_") || name == "test" {
                    out.push(crate::SymbolAttribute::TestMarker);
                }
            }
        }
        // Existing PR8 — decorator harvesting via decorated_definition parent.
        if let Some(parent) = node.parent() {
            if parent.kind() == "decorated_definition" {
                let mut cursor = parent.walk();
                for child in parent.children(&mut cursor) {
                    if child.kind() != "decorator" {
                        continue;
                    }
                    if let Some(name) = extract_decorator_callee(&child, source) {
                        // v1.4 S-001a-prereq AS-008: PEP 698 `@override` /
                        // `@typing.override` decorator → SymbolAttribute::
                        // Override (in addition to the Decorator entry, so
                        // DECORATES edge still emits per existing PR8).
                        // Recognises bare last-segment "override" so both
                        // `@override` and `@typing.override` (qualified)
                        // fire.
                        if is_override_decorator(&name) {
                            out.push(crate::SymbolAttribute::Override);
                        }
                        // Gap 6 / AS-016 — extract argument-list text for
                        // DECORATES edge's decorator_args column. The
                        // decorator node has children (`@`, callee). When
                        // callee is a `call`, its `arguments` child is the
                        // argument_list whose text we capture (paren-stripped).
                        let args = extract_decorator_args(&child, source);
                        out.push(crate::SymbolAttribute::Decorator { name, args });
                    }
                }
            }
        }
        out
    }

    /// PR5c2a — Python params from `function_definition.parameters`. Handles
    /// 4 child kinds:
    /// - `identifier`        → name only (Tools-C2 empty type sentinel)
    /// - `typed_parameter`   → name (first identifier child) + type field
    /// - `default_parameter` → name + default value (no type)
    /// - `typed_default_parameter` → name + type + default value
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if node.kind() != "function_definition" {
            return Vec::new();
        }
        let plist = match node.child_by_field_name("parameters") {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut cursor = plist.walk();
        for child in plist.named_children(&mut cursor) {
            let kind = child.kind();
            match kind {
                "identifier" => {
                    if let Ok(name) = child.utf8_text(source) {
                        out.push(crate::ParsedParam {
                            name: name.trim().to_string(),
                            type_: String::new(),
                            default_value: String::new(),
                        });
                    }
                }
                "typed_parameter" => {
                    // First identifier-kind child is the name; `type` field
                    // holds the annotation.
                    let mut cc = child.walk();
                    let name = child
                        .named_children(&mut cc)
                        .find(|c| c.kind() == "identifier")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let ty = child
                        .child_by_field_name("type")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        out.push(crate::ParsedParam {
                            name,
                            type_: ty,
                            default_value: String::new(),
                        });
                    }
                }
                "default_parameter" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let val = child
                        .child_by_field_name("value")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        out.push(crate::ParsedParam {
                            name,
                            type_: String::new(),
                            default_value: val,
                        });
                    }
                }
                "typed_default_parameter" => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let ty = child
                        .child_by_field_name("type")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let val = child
                        .child_by_field_name("value")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        out.push(crate::ParsedParam {
                            name,
                            type_: ty,
                            default_value: val,
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// PR5b — Python `function_definition.return_type` (only present when
    /// the source has `-> T:` annotation; absent for unannotated defs).
    /// Tools-C2: returning None → indexer maps to `''` empty sentinel.
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "function_definition" {
            return None;
        }
        crate::langs::shared::extract_return_type_by_field(node, source, "return_type")
    }
}

/// Python `@decorator` callee extraction. Three forms:
///   - `@cache` → child kind `identifier` → "cache"
///   - `@app.route(...)` → child kind `attribute` / `dotted_name` → trailing
///   - `@app.api.get(arg)` → child kind `call` → recurse via standard handling
///
/// v1.4 S-001a-prereq AS-008 — recognise PEP 698 `@override` /
/// `@typing.override` decorator. Bare last-segment "override" matches
/// both forms so qualified imports (`@typing.override`, `@T.override`)
/// fire alongside the bare `@override`.
fn is_override_decorator(name: &str) -> bool {
    if name == "override" {
        return true;
    }
    // Qualified form: "<prefix>.override". Use rsplit to grab last segment.
    name.rsplit('.').next() == Some("override")
}

/// Body verbatim from calls.rs::extract_call_name `if kind == "decorator"`
/// branch pre-D4.
fn extract_decorator_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => return child.utf8_text(source).ok().map(|s| s.to_string()),
            "attribute" | "dotted_name" => {
                let text = child.utf8_text(source).ok()?;
                return text.split('.').next_back().map(|s| s.to_string());
            }
            // Decorator wraps a call — recurse via engine's standard handling.
            // Per-lang `decorator` extractor isn't re-entered (child kind is
            // `call`, not `decorator`), so no recursion loop.
            "call" => return crate::calls::extract_standard_callee(&child, source),
            _ => {}
        }
    }
    None
}

/// Gap 6 / AS-016 — extract decorator argument list as raw paren-stripped
/// source text. For `@my_decorator` (no parens) returns "". For
/// `@app.route('/users', methods=['GET'])` returns
/// `'/users', methods=['GET']`. Tools-C14 sanitization applies at indexer
/// CSV-emit time, not here — keep raw to preserve UC-readable form.
fn extract_decorator_args(node: &Node<'_>, source: &[u8]) -> String {
    // decorator > call > arguments. Walk children for `call`, then its
    // `arguments` child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "call" {
            continue;
        }
        if let Some(args_node) = child.child_by_field_name("arguments") {
            if let Ok(text) = args_node.utf8_text(source) {
                // Strip outer parens. argument_list text typically
                // includes the `(...)`; trim them.
                let t = text.trim();
                let stripped = t.strip_prefix('(').unwrap_or(t);
                let stripped = stripped.strip_suffix(')').unwrap_or(stripped);
                return stripped.trim().to_string();
            }
        }
    }
    String::new()
}
