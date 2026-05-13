//! C# `LanguageSpec`. Grammar: `tree-sitter-c-sharp` 0.23 (pinned in
//! Cargo.toml + Cargo.lock per AS-016).
//!
//! v1.1-M4 sub-units:
//!   - S-003a: skeleton (node-kind metadata + family + empty extractors)
//!   - S-003b: AS-009 CALLS happy path + parse tolerance + csharp-tiny
//!     fixture (Lang-C2)
//!   - S-003c: AS-010 EXTENDS + interfaces + AS-011 partial classes +
//!     IMPORTS (`using_directive` plain/static/alias) + [Inject]/[Required]
//!     REFERENCES (Lang-C7 PARTIAL — Blazor uses `[Inject]`, ASP.NET Core
//!     mostly constructor-injection but field DI exists in Blazor and
//!     Razor pages).

use crate::references::{ParsedReference, RefKind};
use crate::{CalleeExtractor, LangFamily, LanguageSpec, RefEmitter};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct CSharpLang;

// S-003b — `object_creation_expression` shape is `[new] [identifier|generic_name|qualified_name] [argument_list]`.
// Default `extract_standard_callee` reads child(0) which is the `new`
// token — wrong. Override to walk past `new` and capture the type name.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] =
    &[("object_creation_expression", extract_object_creation_callee)];

// S-003c (Lang-C7 PARTIAL) — `[Inject]` / `[Required]` attributed field or
// property → REFERENCES edge to declared type. Wired on both
// `field_declaration` and `property_declaration` so plain DI fields
// (`[Inject] private IRepo _repo;`) and Blazor `[Inject]` properties
// (`[Inject] public IRepo Repo { get; set; }`) both surface.
//
// Mirrors Java field_declaration emit-once semantics.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[
    ("field_declaration", extract_attributed_field_ref),
    ("property_declaration", extract_attributed_property_ref),
];

// AS-016 checklist — AST node kinds tree-sitter-c-sharp 0.23 emits per
// category. Probed against canonical fixtures (see grammar_drift.rs).
//
// C# emits distinct top-level kinds for each declaration form (unlike
// Kotlin which lumps class/interface/enum/object into class_declaration).
// SymbolKind classification (`classify_kind`) maps:
//   - class_declaration → Class
//   - interface_declaration → Interface
//   - enum_declaration → Enum
//   - struct_declaration → Struct
//   - record_declaration → Class (no Record SymbolKind variant)
//   - delegate_declaration → Other (no Delegate variant)
//   - method_declaration → Method
//   - constructor_declaration → Method
const SYMBOLS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "struct_declaration",
    "record_declaration",
    "delegate_declaration",
    "method_declaration",
    "constructor_declaration",
];
// `using` directives. Three syntactic forms all parse to `using_directive`:
//   - `using System;`             — plain
//   - `using static System.Math;` — static (members of type into scope)
//   - `using F = System.Foo;`     — alias
const IMPORTS: &[&str] = &["using_directive"];
// `invocation_expression` covers method calls. `object_creation_expression`
// covers `new T(...)` constructor calls.
const CALLS: &[&str] = &["invocation_expression", "object_creation_expression"];
// EXTENDS: any nominal type with a `base_list` may have inheritance
// relationships. `record_declaration` also accepts a base_list in C# 9+.
const EXTENDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "struct_declaration",
    "record_declaration",
];

impl LanguageSpec for CSharpLang {
    fn lang(&self) -> Lang {
        Lang::CSharp
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_c_sharp::LANGUAGE.into()
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
        LangFamily::StaticManaged
    }

    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }

    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }

    /// C# EXTENDS extraction (AS-010).
    ///
    /// Tree-sitter shape:
    /// ```text
    /// class_declaration "class Admin : User, IPrintable, ICloneable"
    ///   ├── base_list ": User, IPrintable, ICloneable"
    ///   │     ├── identifier "User"
    ///   │     ├── identifier "IPrintable"
    ///   │     └── identifier "ICloneable"
    /// ```
    /// `qualified_name` (e.g. `System.User`) → trailing identifier.
    /// `generic_name` (e.g. `Container<int>`) → first identifier child.
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "base_list" {
                continue;
            }
            let mut c2 = child.walk();
            for spec in child.children(&mut c2) {
                if let Some(name) = base_name_from_node(&spec, source) {
                    bases.push(name);
                }
            }
        }
        bases
    }

    /// C# IMPORTS — `using_directive` covers three forms:
    /// - `using System;`             → target_path = "System"
    /// - `using System.Math;`        → target_path = "System.Math"
    /// - `using static System.Math;` → target_path = "System.Math" (path portion)
    /// - `using F = System.Foo;`     → target_path = "System.Foo" (RHS)
    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        // Collect all path-eligible children; the LAST one is the
        // target path (for alias form, RHS comes after `=`; for plain,
        // there's only one).
        let mut last_path: Option<String> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "qualified_name" | "generic_name" | "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        last_path = Some(text.to_string());
                    }
                }
                _ => {}
            }
        }
        last_path
    }

    /// C# imported-name extraction.
    ///
    /// - `using System;`              → `["System"]` (last segment)
    /// - `using System.Collections;`  → `["Collections"]`
    /// - `using static System.Math;`  → `["Math"]` (containing type)
    /// - `using F = System.Foo;`      → `["F"]` (LOCAL alias name)
    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut has_eq = false;
        let mut alias_lhs: Option<String> = None;
        let mut path_text: Option<String> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "=" => has_eq = true,
                "identifier" if !has_eq => {
                    // Could be: (a) LHS of alias, (b) the only path token
                    // of `using System;`. Disambiguate by checking later.
                    if let Ok(text) = child.utf8_text(source) {
                        alias_lhs = Some(text.to_string());
                    }
                }
                "qualified_name" | "generic_name" => {
                    if let Ok(text) = child.utf8_text(source) {
                        path_text = Some(text.to_string());
                    }
                }
                "identifier" if has_eq && path_text.is_none() => {
                    // RHS of alias is normally qualified_name; but if
                    // `using F = SomeType;` (single identifier RHS),
                    // capture as path.
                    if let Ok(text) = child.utf8_text(source) {
                        path_text = Some(text.to_string());
                    }
                }
                _ => {}
            }
        }

        if has_eq {
            // Alias form: LOCAL name is the LHS.
            return alias_lhs.into_iter().collect();
        }
        // Plain or static: last segment of path. If path_text is None
        // (single-identifier `using System;`), fall back to alias_lhs.
        let path = path_text.or(alias_lhs).unwrap_or_default();
        if path.is_empty() {
            return Vec::new();
        }
        path.rsplit('.')
            .next()
            .map(|s| vec![s.to_string()])
            .unwrap_or_default()
    }

    /// Gap 4 — C# attributes. Walk method_declaration children: `modifier`
    /// keyword "override"/"static" → SymbolAttribute::Override/Static.
    /// `attribute_list > attribute > identifier` → Annotation(name).
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::SymbolAttribute> {
        if !matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "modifier" => {
                    if let Ok(t) = child.utf8_text(source) {
                        match t.trim() {
                            "override" => attrs.push(crate::SymbolAttribute::Override),
                            "static" => attrs.push(crate::SymbolAttribute::Static),
                            "async" => attrs.push(crate::SymbolAttribute::Async),
                            _ => {}
                        }
                    }
                }
                "attribute_list" => {
                    let mut ac = child.walk();
                    for attr in child.named_children(&mut ac) {
                        if attr.kind() == "attribute" {
                            // attribute > identifier name
                            let mut tc = attr.walk();
                            let name = attr
                                .named_children(&mut tc)
                                .find(|c| matches!(c.kind(), "identifier" | "qualified_name"))
                                .and_then(|n| n.utf8_text(source).ok())
                                .map(|s| s.rsplit('.').next().unwrap_or(s).to_string());
                            if let Some(n) = name {
                                attrs.push(crate::SymbolAttribute::Annotation(n));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        attrs
    }

    /// Gap 3 — C# return type. tree-sitter-c-sharp `method_declaration` has
    /// positional children: `modifier`s → return-type node → `identifier`
    /// (name) → `parameter_list`. Return type is the named child immediately
    /// before the name identifier (which is also right before parameter_list).
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "method_declaration" {
            return None;
        }
        let mut cursor = node.walk();
        let mut prev: Option<Node<'_>> = None;
        for child in node.named_children(&mut cursor) {
            if child.kind() == "parameter_list" {
                // Walk back: child immediately before parameter_list is name.
                // Return type is one before that.
                break;
            }
            // Track the second-to-last named child before parameter_list.
            // Two-step lookback via swapping.
            if prev.is_some() {
                // prev now becomes 2-back when we advance.
            }
            prev = Some(child);
        }
        // Iterate again to find name + return-type via index.
        let mut cursor = node.walk();
        let named: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        let plist_idx = named.iter().position(|n| n.kind() == "parameter_list")?;
        if plist_idx < 2 {
            return None;
        }
        // Return type is named[plist_idx - 2] (name is plist_idx - 1).
        let rt_node = named[plist_idx - 2];
        // Skip modifier nodes — they're separate kind, not type kinds.
        if rt_node.kind() == "modifier" {
            return None;
        }
        let text = rt_node.utf8_text(source).ok()?.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    /// PR5c2b — C# modifiers: each `modifier` named child of method_declaration.
    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if !matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return Vec::new();
        }
        let mut mods = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "modifier" {
                if let Ok(t) = child.utf8_text(source) {
                    mods.push(t.trim().to_string());
                }
            }
        }
        mods
    }

    /// PR5c2b — C# params: `parameter_list` field with `parameter` children.
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if !matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return Vec::new();
        }
        crate::langs::shared::extract_params_by_container(node, source, "parameters")
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

/// Map a base_list child to a base name. `identifier` → text directly;
/// `qualified_name` → trailing segment; `generic_name` → first identifier
/// child (raw type, generic args stripped).
fn base_name_from_node(node: &Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node.utf8_text(source).ok().map(str::to_string),
        "qualified_name" => {
            let text = node.utf8_text(source).ok()?;
            text.rsplit('.').next().map(str::to_string)
        }
        "generic_name" => {
            let mut cursor = node.walk();
            for c in node.children(&mut cursor) {
                if c.kind() == "identifier" {
                    return c.utf8_text(source).ok().map(str::to_string);
                }
            }
            None
        }
        _ => None,
    }
}

/// `new User("a")` → "User"; `new System.User()` → "User" (last segment);
/// `new List<int>()` → "List" (raw type name from generic_name).
fn extract_object_creation_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = base_name_from_node(&child, source) {
            return Some(name);
        }
    }
    None
}

/// AS-004-equiv (Lang-C7 PARTIAL) — emit one ParsedReference per
/// attributed field. Mirrors Java emit-once-per-field.
fn extract_attributed_field_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    if !has_attribute(node) {
        return;
    }
    // field_declaration → variable_declaration → first type-bearing child
    let Some(var_decl) = first_child_of_kind(node, "variable_declaration") else {
        return;
    };
    let Some(target_name) = first_type_name_in(&var_decl, source) else {
        return;
    };
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name,
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::AnnotatedFieldType,
    });
}

/// AS-004-equiv (Lang-C7 PARTIAL) — emit one ParsedReference per
/// attributed property.
fn extract_attributed_property_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    if !has_attribute(node) {
        return;
    }
    // property_declaration directly carries the type as the first
    // type-bearing child after attribute_list / modifier(s).
    let Some(target_name) = first_type_name_in(node, source) else {
        return;
    };
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name,
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::AnnotatedFieldType,
    });
}

/// True if `node` has at least one `attribute_list` child.
fn has_attribute(node: &Node<'_>) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            return true;
        }
    }
    false
}

fn first_child_of_kind<'t>(node: &Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    #[allow(clippy::manual_find)]
    {
        for child in node.children(&mut cursor) {
            if child.kind() == kind {
                return Some(child);
            }
        }
        None
    }
}

/// Return the trailing identifier of the FIRST type-bearing child
/// (`identifier` / `qualified_name` / `generic_name` / `predefined_type`)
/// of `node`. Skips `attribute_list` / `modifier` tokens.
fn first_type_name_in(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute_list" | "modifier" | "ref_modifier" => continue,
            "identifier" | "qualified_name" | "generic_name" => {
                return base_name_from_node(&child, source);
            }
            "predefined_type" => {
                return child.utf8_text(source).ok().map(str::to_string);
            }
            _ => {}
        }
    }
    None
}
