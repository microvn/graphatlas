//! Rust `LanguageSpec`. Grammar: `tree-sitter-rust` 0.24.

use crate::references::{is_clean_ident, is_type_ident};
use crate::{CalleeExtractor, LangFamily, LanguageSpec, ParsedReference, RefEmitter, RefKind};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct RustLang;

const SYMBOLS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
];
const IMPORTS: &[&str] = &["use_declaration"];
const CALLS: &[&str] = &["call_expression", "macro_invocation"];
const EXTENDS: &[&str] = &["impl_item"];

// S-005a D3 — `call_expression` arg-identifier emitter, migrated from
// references.rs. Was hardcoded as `if matches!(lang, Lang::Rust) { … }`
// inside the engine; now lives here and is registered via `ref_emitters()`.
//
// 2026-04-28 — added `type_identifier` emitter for type-position uses
// (let x: Foo, -> Foo, Vec<Foo>, function arg types). Closes the
// dead_code FP gap where ga's indexer missed Rust type-position
// references that Hd-ast's raw extraction picked up.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[
    ("call_expression", emit_call_arg_fn_pointer),
    ("type_identifier", emit_type_position),
];

// S-005a D4 — `macro_invocation` callee extractor, migrated from calls.rs.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] =
    &[("macro_invocation", extract_macro_invocation_callee)];

impl LanguageSpec for RustLang {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_rust::LANGUAGE.into()
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

    /// rust-poc/src/main.rs:324-335 — on `impl_item` with a `trait` field,
    /// record the trait name (strip generics).
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if node.kind() != "impl_item" {
            return Vec::new();
        }
        let Some(trait_node) = node.child_by_field_name("trait") else {
            return Vec::new();
        };
        let Ok(text) = trait_node.utf8_text(source) else {
            return Vec::new();
        };
        let name = text.split('<').next().unwrap_or(text).trim();
        if name.len() > 1 {
            vec![name.to_string()]
        } else {
            Vec::new()
        }
    }

    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        // Hard gate on node kind: a Rust "import" is only ever `use_declaration`.
        // Previously this function fell through to raw `utf8_text` for any node
        // kind (bug H-1 from S-004 audit), returning garbage for non-use nodes.
        if node.kind() != "use_declaration" {
            return None;
        }
        // Prefer the shared helper (scoped_identifier / identifier), fall back
        // to stripping the `use ` prefix from the raw text — now safely scoped
        // to `use_declaration` only.
        if let Some(s) = crate::langs::shared::extract_import_path_default(node, source) {
            return Some(s);
        }
        let text = node.utf8_text(source).ok()?;
        let stripped = text
            .trim()
            .strip_prefix("use ")
            .unwrap_or(text)
            .trim_end_matches(';')
            .trim();
        if stripped.is_empty() {
            None
        } else {
            Some(stripped.to_string())
        }
    }

    fn family(&self) -> LangFamily {
        LangFamily::SystemsRust
    }

    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        crate::imports::rust_imported_names(node, source)
    }

    fn extract_imported_aliases(&self, node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
        crate::imports::rust_imported_aliases(node, source)
    }

    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }

    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }

    /// Gap 5 — Rust attributes. Walk function_item siblings for outer
    /// `attribute_item` nodes preceding the fn (`#[test]`, `#[tokio::test]`,
    /// `#[cfg(test)]`). Walker passes the function_item node; tree-sitter-rust
    /// places attribute_item nodes as preceding siblings INSIDE the parent
    /// declaration list — they appear as named children of source_file/mod
    /// alongside the function_item, NOT children of function_item itself.
    /// We walk the function_item's previous siblings to find them.
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        if node.kind() != "function_item" {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        // Async modifier appears as `function_modifiers > async` per existing
        // convention — but we already have extract_modifiers wired separately.
        // Map async modifier → SymbolAttribute::Async for is_async bool.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "function_modifiers" {
                let mut cc = child.walk();
                for m in child.children(&mut cc) {
                    if let Ok(t) = m.utf8_text(source) {
                        if t.trim() == "async" {
                            attrs.push(crate::SymbolAttribute::Async);
                        }
                    }
                }
            }
        }
        // v1.4 S-001a-prereq AS-007 — Rust trait-impl methods emit
        // SymbolAttribute::Override. Walk ancestors: function_item is
        // inside `declaration_list` whose parent is `impl_item`. If
        // impl_item carries a `trait` field, the function is overriding
        // that trait's method definition. Inherent `impl Type {}` (no
        // trait field) → no Override. Mirrors Tools-C18 IMPLEMENTS branch
        // for the class-level witness.
        let mut up = node.parent();
        while let Some(parent) = up {
            if parent.kind() == "impl_item" {
                if parent.child_by_field_name("trait").is_some() {
                    attrs.push(crate::SymbolAttribute::Override);
                }
                break;
            }
            up = parent.parent();
        }
        // Outer attributes: walk preceding siblings looking for attribute_item.
        let mut sib = node.prev_sibling();
        while let Some(s) = sib {
            if s.kind() == "attribute_item" {
                if let Ok(text) = s.utf8_text(source) {
                    let t = text.trim();
                    // `#[test]` / `#[tokio::test]` / `#[cfg(test)]`
                    if t == "#[test]"
                        || t.starts_with("#[test")
                        || t.contains("::test]")
                        || t.contains("(test)")
                    {
                        attrs.push(crate::SymbolAttribute::TestMarker);
                    }
                }
                sib = s.prev_sibling();
            } else if s.kind() == "line_comment" || s.kind() == "block_comment" {
                sib = s.prev_sibling();
            } else {
                break;
            }
        }
        attrs
    }

    /// PR5c2a — Rust modifiers from `visibility_modifier` (pub / pub(crate))
    /// + `function_modifiers` block (async / unsafe / const / extern).
    /// Children traversed in source order so emission is deterministic.
    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if node.kind() != "function_item" {
            return Vec::new();
        }
        let mut mods = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "visibility_modifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        let t = text.trim();
                        if !t.is_empty() {
                            mods.push(t.to_string());
                        }
                    }
                }
                "function_modifiers" => {
                    let mut cc = child.walk();
                    for m in child.children(&mut cc) {
                        if let Ok(t) = m.utf8_text(source) {
                            let t = t.trim();
                            if !t.is_empty() {
                                mods.push(t.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        mods
    }

    /// PR5c2a — Rust params: walk `parameters` field, handle `self_parameter`
    /// (yields `name = "self"`, `type_ = raw text` like `&self` / `self` /
    /// `&mut self`) + ordinary `parameter` (field `pattern` → name; field
    /// `type` → type). Default values are not part of Rust fn signatures —
    /// always empty.
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if node.kind() != "function_item" {
            return Vec::new();
        }
        let plist = match node.child_by_field_name("parameters") {
            Some(p) => p,
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut cursor = plist.walk();
        for child in plist.named_children(&mut cursor) {
            match child.kind() {
                "self_parameter" => {
                    let raw = child.utf8_text(source).unwrap_or("self").trim();
                    out.push(crate::ParsedParam {
                        name: "self".to_string(),
                        type_: raw.to_string(),
                        default_value: String::new(),
                    });
                }
                "parameter" => {
                    let name = child
                        .child_by_field_name("pattern")
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
                _ => {}
            }
        }
        out
    }

    /// PR5b — Rust `function_item.return_type` field text (already strips `->`).
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "function_item" {
            return None;
        }
        crate::langs::shared::extract_return_type_by_field(node, source, "return_type")
    }

    /// PR4 / AS-005(a) — `impl_item` has `type` field but no `name` field,
    /// so the default walker enclosing-tracker misses it. Extract the
    /// receiver type so `fn fmt` inside `impl Display for Foo` carries
    /// `EnclosingScope::Class("Foo")`. Trait-side (`Display`) is captured
    /// by `extract_bases` for EXTENDS edges; receiver-side is what
    /// disambiguates qualified_name.
    fn container_name_fallback(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "impl_item" {
            return None;
        }
        let ty = node.child_by_field_name("type")?;
        let text = ty.utf8_text(source).ok()?;
        // Strip generics for stability: `Foo<T>` → `Foo`.
        let bare = text.split('<').next().unwrap_or(text).trim();
        if bare.is_empty() {
            None
        } else {
            Some(bare.to_string())
        }
    }

    /// PR4 / Tools-C1 — Rust uses `::` path separator. Receiver type from
    /// `impl Foo { fn bar }` lives in `EnclosingScope::Class("Foo")` —
    /// covers AS-005 collision class (a) by construction (`Foo::bar` ≠
    /// `Bar::bar`). Macro-expanded same-name symbols (class d) and
    /// overloads (class e) collide on this format and rely on indexer
    /// AS-006 `#dup<N>` dedup for uniqueness.
    fn format_qualified_name(
        &self,
        name: &str,
        enclosing: Option<&crate::EnclosingScope>,
    ) -> String {
        match enclosing {
            Some(scope) => format!("{}::{}", scope.as_str(), name),
            None => name.to_string(),
        }
    }
}

/// Rust `macro_invocation` callee — `println!`, `vec!`, `assert_eq!` etc.
/// Strip trailing `!` from the macro name. 1-char names rejected as noise.
///
/// Body verbatim from calls.rs::extract_call_name `if kind == "macro_invocation"`
/// branch pre-D4.
fn extract_macro_invocation_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let name_node = node.child(0)?;
    let text = name_node.utf8_text(source).ok()?;
    let name = text.trim_end_matches('!');
    if name.len() > 1 {
        Some(name.to_string())
    } else {
        None
    }
}

/// infra:S-001 AS-002 — Rust `call_expression` arguments that are bare
/// `identifier` nodes (fn pointers passed by name).
///
/// Skips the callee itself (`child_by_field_name("function")`) — we don't
/// want `foo` in `foo(bar)` to be logged as a fn-pointer-arg reference of
/// `foo`. Only the `bar` args are candidates.
///
/// Body verbatim from references.rs::emit_rust_call_arg_identifiers pre-D3.
/// M-1 review (2026-04-24): heuristic over-emits — e.g. `log(msg, level)`
/// with `msg: &str` surfaces `msg` as a FnPointerArg. Indexer-layer
/// `symbol_by_file_name` / `symbol_by_name` lookup drops targets that
/// don't resolve to a defined non-external symbol. Accepted as bounded
/// noise for v1.1; revisit if Phase B semantic bench shows impact.
fn emit_call_arg_fn_pointer(
    call: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let Some(args) = call.child_by_field_name("arguments") else {
        return;
    };
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() != "identifier" {
            continue;
        }
        let Ok(name) = child.utf8_text(source) else {
            continue;
        };
        if !is_clean_ident(name) {
            continue;
        }
        out.push(ParsedReference {
            enclosing_symbol: enclosing.clone(),
            target_name: name.to_string(),
            ref_site_line: (child.start_position().row as u32) + 1,
            ref_kind: RefKind::FnPointerArg,
        });
    }
}

/// 2026-04-28 — Rust `type_identifier` emitter.
///
/// tree-sitter-rust uses a dedicated `type_identifier` node for type
/// names — captures `Foo` in `let x: Foo`, `-> Foo`, `Vec<Foo>`,
/// `&Foo`, function arg types, etc. The same name appearing as a
/// value (`Foo::new()`) uses `identifier`, so this emitter is
/// type-position-only by grammar.
///
/// Skip when this `type_identifier` is the NAME slot of its own
/// `(struct|enum|trait|type|union)_item` — i.e. the definition site —
/// to avoid recursive self-edges.
///
/// Stopwords: Rust primitive types. Without this filter, every `i32`/
/// `bool`/`String`/etc. use would explode the universe.
fn emit_type_position(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let Ok(name) = node.utf8_text(source) else {
        return;
    };
    if !is_type_ident(name) || is_rust_primitive(name) {
        return;
    }
    if let Some(parent) = node.parent() {
        let parent_kind = parent.kind();
        let is_def_parent = matches!(
            parent_kind,
            "struct_item" | "enum_item" | "trait_item" | "type_item" | "union_item"
        );
        if is_def_parent {
            // The definition's name slot is `child_by_field_name("name")`.
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

fn is_rust_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "String"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "Self"
            | "Box"
            | "Vec"
            | "Option"
            | "Result"
            | "HashMap"
            | "HashSet"
            | "BTreeMap"
            | "BTreeSet"
            | "Rc"
            | "Arc"
    )
}
