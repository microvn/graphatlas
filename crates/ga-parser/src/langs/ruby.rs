//! Ruby `LanguageSpec`. Grammar: `tree-sitter-ruby` 0.23 (pinned in
//! Cargo.toml + Cargo.lock per AS-016).
//!
//! v1.1-M4 sub-units:
//!   - S-004a: skeleton (node-kind metadata + family + empty extractors)
//!   - S-004b: AS-012 CALLS happy path + parse tolerance + ruby-tiny
//!     fixture (Lang-C2)
//!   - S-004c: AS-013 Rails convention (canonical is_test_path already
//!     covers `_spec.rb` / `_test.rb` post §4.2.6 refactor) + AS-014
//!     `define_method` polymorphic confidence (Tools-C11) + EXTENDS
//!     `class X < Y` superclass field.
//!
//! Ruby paradigm decisions per §0.6 of dataset-for-new-language.md:
//!   - 0.6-D field-annotation DI: **NO** (Rails uses constructor injection;
//!     `attr_*` macros are setters, not DI markers). Lang-C7 N/A — no
//!     `RefKind::AnnotatedFieldType` emit. Spec sub-doc records the
//!     explicit skip so future readers don't think it's a missed impl.
//!   - 0.6-I polymorphic dispatch: YES heavy (`define_method`,
//!     `method_missing`). Confidence ≤0.6 per Tools-C11 — handled in
//!     S-004c via SymbolAttribute / confidence layer.
//!   - 0.6-O wildcard imports: NO (no `require *` form).
//!   - family: `DynamicScripting` (groups Python/Ruby/PHP/Lua duck-typed
//!     metaprogramming langs).

use crate::{CalleeExtractor, LangFamily, LanguageSpec, ParsedSymbol, SymbolAttribute};
use ga_core::{Lang, SymbolKind};
use tree_sitter::{Language, Node};

pub struct RubyLang;

// S-004b — tree-sitter-ruby `call` node fields are `receiver` / `method` /
// `arguments` (NOT `function` — that's the default extractor's preference).
// `extract_standard_callee` falls back to `child(0)`, which for receiver-form
// calls (`Base.lookup(id)`) is the receiver `Base`, not the method `lookup`.
// Override emits the method name in receiver-form, falls back to bare-call
// child(0) identifier when no `method` field is present.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[("call", extract_ruby_call_callee)];

// AS-016 checklist — AST node kinds tree-sitter-ruby 0.23 emits per
// category. Probed against canonical fixtures (see grammar_drift.rs).
//
// SymbolKind classification (`classify_kind`) maps:
//   - `class`  → Class (substring match on "class")
//   - `module` → Other (no Module SymbolKind variant; `classify_kind` falls
//      through; downstream surfaces as Other which is acceptable for now)
//   - `method` / `singleton_method` → Method
const SYMBOLS: &[&str] = &["class", "module", "method", "singleton_method"];

// Ruby has NO static import statement — `require` / `require_relative` are
// runtime method calls (parsed as `call` / `command` nodes). Static imports
// are not in scope for v1.1-M4 S-004 (would need a per-call inspection
// pass at the indexer layer, not the parser layer). Empty list keeps the
// engine from emitting bogus IMPORTS edges.
const IMPORTS: &[&str] = &[];

// tree-sitter-ruby 0.23 emits `call` for ALL invocation forms — receiver
// calls (`obj.method(args)`), bare calls (`require 'foo'`), parenless
// macros (`attr_accessor :name`), constant calls (`Base.lookup(id)`).
// Verified via AST probe of canonical fixture source. No separate
// `command` / `command_call` kinds in this grammar.
const CALLS: &[&str] = &["call"];

// `class X < Y` → class node carries a `superclass` field. Ruby has no
// multiple-inheritance form (mixins via `include` are method calls, not
// structural inheritance).
const EXTENDS: &[&str] = &["class"];

impl LanguageSpec for RubyLang {
    fn lang(&self) -> Lang {
        Lang::Ruby
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_ruby::LANGUAGE.into()
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

    fn family(&self) -> LangFamily {
        LangFamily::DynamicScripting
    }

    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }

    /// AS-013-equiv — Ruby `class X < Y` superclass extraction.
    ///
    /// Tree-sitter shape:
    /// ```text
    /// class "class Admin < ApplicationController::Base"
    ///   ├── "class"
    ///   ├── constant "Admin"
    ///   ├── superclass "< ApplicationController::Base"
    ///   │     ├── "<"
    ///   │     └── scope_resolution "ApplicationController::Base"
    ///   │           ├── constant "ApplicationController"
    ///   │           ├── "::"
    ///   │           └── constant "Base"
    ///   └── ...
    /// ```
    ///
    /// Strategy: walk `class` children, find `superclass` field, then walk
    /// its children for `constant` (text directly) or `scope_resolution`
    /// (rsplit `::` → trailing constant). Ruby has no multiple inheritance
    /// — at most one EXTENDS edge per class.
    ///
    /// Mixins via `include Foo` / `extend Foo` are method calls, not
    /// structural inheritance — handled by REFERENCES traversal at indexer
    /// layer, not extract_bases.
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "superclass" {
                continue;
            }
            let mut sc = child.walk();
            for sc_child in child.children(&mut sc) {
                match sc_child.kind() {
                    "constant" => {
                        if let Ok(text) = sc_child.utf8_text(source) {
                            bases.push(text.to_string());
                        }
                    }
                    "scope_resolution" => {
                        if let Ok(text) = sc_child.utf8_text(source) {
                            if let Some(last) = text.rsplit("::").next() {
                                bases.push(last.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        bases
    }

    /// AS-014 — `define_method(:foo) { ... }` / `define_method :foo do ... end`
    /// emits a synthetic ParsedSymbol with confidence 0.6 per Tools-C11.
    ///
    /// Tree-sitter shape:
    /// ```text
    /// call "define_method(:dyn) { ... }"
    ///   ├── identifier "define_method"     ← child(0); marker
    ///   ├── argument_list                  ← contains the symbol literal
    ///   │     └── simple_symbol ":dyn"     ← strip leading `:` → "dyn"
    ///   └── block | do_block               ← (ignored)
    /// ```
    ///
    /// Confidence 0.6 reflects "tree-sitter can't resolve string-named
    /// symbols at parse time" — the symbol exists statically in the source
    /// but its name is data, not syntax. Indexer-layer warning surfaces in
    /// `ga_symbols` per AS-014 Then clause.
    fn extract_synthetic_symbol(&self, node: &Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
        if node.kind() != "call" {
            return None;
        }
        // Receiver-form `obj.define_method(...)` is a runtime call on an
        // instance; it does NOT statically define a method on the
        // enclosing class. The bare-call form has no `receiver` field.
        if node.child_by_field_name("receiver").is_some() {
            return None;
        }
        // The call's "method" field (or child(0) fallback) must be the
        // bare identifier `define_method`. Tree-sitter-ruby tags the
        // function-side identifier as `method` field on every call,
        // including parenless bare calls.
        let method = node
            .child_by_field_name("method")
            .or_else(|| node.child(0))?;
        if method.kind() != "identifier" {
            return None;
        }
        if method.utf8_text(source).ok()? != "define_method" {
            return None;
        }
        // Find the first simple_symbol argument inside argument_list.
        let args = node.child_by_field_name("arguments")?;
        let mut cursor = args.walk();
        for arg in args.children(&mut cursor) {
            if arg.kind() == "simple_symbol" {
                let text = arg.utf8_text(source).ok()?;
                let name = text.strip_prefix(':').unwrap_or(text).to_string();
                if name.is_empty() {
                    return None;
                }
                return Some(ParsedSymbol {
                    name,
                    kind: SymbolKind::Method,
                    line: (node.start_position().row as u32) + 1,
                    // Synthetic — reuse start line; line span = 1.
                    line_end: (node.end_position().row as u32) + 1,
                    enclosing: None,
                    attributes: vec![SymbolAttribute::Decorator {
                        name: "define_method".to_string(),
                        args: String::new(),
                    }],
                    confidence: 0.6,
                    // Synthetic — block-arg arity unknown without runtime.
                    arity: None,
                    // Ruby has no static return types.
                    return_type: None,
                    modifiers: Vec::new(),
                    params: Vec::new(),
                });
            }
        }
        None
    }

    /// Gap 5 — Ruby Test::Unit / Minitest convention: `def test_*`.
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        if !matches!(node.kind(), "method" | "singleton_method") {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source) {
                if name.starts_with("test_") {
                    attrs.push(crate::SymbolAttribute::TestMarker);
                }
            }
        }
        attrs
    }

    /// PR5c2b — Ruby params: `parameters` field on `method` node. Children:
    /// `identifier` (simple), `optional_parameter` (with default), `splat_parameter`,
    /// `keyword_parameter`, `hash_splat_parameter`, `block_parameter`. Ruby has
    /// no static types — all `type` empty (Tools-C2 sentinel).
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if !matches!(node.kind(), "method" | "singleton_method") {
            return Vec::new();
        }
        let plist = match node
            .child_by_field_name("parameters")
            .or_else(|| node.child_by_field_name("method_parameters"))
        {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut cursor = plist.walk();
        for child in plist.named_children(&mut cursor) {
            let kind = child.kind();
            match kind {
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        out.push(crate::ParsedParam {
                            name: text.trim().to_string(),
                            type_: String::new(),
                            default_value: String::new(),
                        });
                    }
                }
                "optional_parameter" => {
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
                "splat_parameter"
                | "hash_splat_parameter"
                | "block_parameter"
                | "keyword_parameter" => {
                    // Find first identifier descendant.
                    let mut cc = child.walk();
                    let name = child
                        .named_children(&mut cc)
                        .find(|c| c.kind() == "identifier")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        out.push(crate::ParsedParam {
                            name,
                            type_: String::new(),
                            default_value: String::new(),
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// PR4 / Tools-C1 — Ruby uses `#` separator for instance methods. Spec
    /// distinguishes `Class.classmethod` vs `Class#instance_method`; PR4
    /// baseline ships `#` for all (class-method detection deferred —
    /// indistinguishable from instance methods at walker time without
    /// scanning for `self.` prefix or `class << self` blocks).
    fn format_qualified_name(
        &self,
        name: &str,
        enclosing: Option<&crate::EnclosingScope>,
    ) -> String {
        match enclosing {
            Some(scope) => format!("{}#{}", scope.as_str(), name),
            None => name.to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

/// AS-012 — Ruby `call` callee extraction.
///
/// Tree-sitter shape:
/// ```text
/// call "Base.lookup(id)"
///   ├── receiver: constant "Base"
///   ├── "."
///   ├── method:   identifier "lookup"
///   └── arguments: argument_list "(id)"
///
/// call "check(name)"
///   ├── method:    identifier "check"   (no receiver)
///   └── arguments: argument_list "(name)"
///
/// call "bar"      (no parens, no args; Ruby parses as call when used in
///                  expression position — e.g. `def foo; bar; end`)
///   └── identifier "bar"
/// ```
///
/// Strategy:
///   1. `method` field → trailing identifier (handles receiver + non-receiver
///      forms uniformly; `obj.foo` / `Class.foo` / `foo(...)` all expose the
///      method name here).
///   2. Fallback: first `identifier` child (bare bodyless call without
///      `method` field).
///   3. Generic-name / scoped name strip: rsplit on `.` for safety on
///      qualified text — Ruby's grammar already structures qualified
///      receivers, so this is defensive only.
fn extract_ruby_call_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(method) = node.child_by_field_name("method") {
        if let Ok(text) = method.utf8_text(source) {
            if !text.is_empty() {
                return Some(text.rsplit('.').next().unwrap_or(text).to_string());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(text) = child.utf8_text(source) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}
