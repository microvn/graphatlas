//! PHP `LanguageSpec`. Grammar: `tree-sitter-php` 0.24.2 (pinned per
//! v1.2-Lang-C9 in `crates/ga-parser/grammar-pins.toml`).
//!
//! v1.2 S-001 implements:
//! - AS-001 CALLS (5 dispatch kinds — member_call, function_call,
//!   nullsafe_member_call, scoped_call, object_creation)
//! - AS-002 IMPORTS (namespace_definition + namespace_use_declaration, 5 shapes)
//! - AS-003 EXTENDS (base_clause + class_interface_clause + body trait use_declaration)
//! - AS-004 DI REFERENCES (#[Required] et al on property_declaration → AnnotatedFieldType)
//! - AS-005 parse tolerance (R12 — never panic)
//!
//! Node-kinds verified per docs/_archived/2026-05-13-php-node-kinds.md.

use crate::references::{ParsedReference, RefKind};
use crate::{CalleeExtractor, LangFamily, LanguageSpec, RefEmitter};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct PhpLang;

const SYMBOLS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
    "method_declaration",
    "function_definition",
    "property_declaration",
];
const IMPORTS: &[&str] = &["namespace_definition", "namespace_use_declaration"];
const CALLS: &[&str] = &[
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "object_creation_expression",
];
const EXTENDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
];

/// DI-style attribute names (Lang-C7). Hardcoded inside php.rs per Java/C#
/// precedent — no trait method exposure, no Lang-C4 ripple.
const PHP_DI_ATTRIBUTES: &[&str] = &[
    "Required",
    "Inject",
    "Autowire",
    "AutowireDecorated",
    "AutowireIterator",
    "AutowireLocator",
];

const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[
    ("function_call_expression", extract_function_call_callee),
    ("member_call_expression", extract_member_call_callee),
    (
        "nullsafe_member_call_expression",
        extract_member_call_callee,
    ),
    ("scoped_call_expression", extract_member_call_callee),
    ("object_creation_expression", extract_object_creation_callee),
];

const REF_EMITTERS: &[(&str, RefEmitter)] =
    &[("property_declaration", extract_annotated_property_ref)];

impl LanguageSpec for PhpLang {
    fn lang(&self) -> Lang {
        Lang::Php
    }
    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_php::LANGUAGE_PHP.into()
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
        // PHP grouped with Python/Ruby/Lua per lib.rs:65. Per-call-site hybrid
        // confidence deferred to v1.3 (Tools-C11 wire-in cross-lang spec).
        LangFamily::DynamicScripting
    }
    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        CALLEE_EXTRACTORS
    }
    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }

    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        match node.kind() {
            "namespace_definition" => node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(str::to_string),
            "namespace_use_declaration" => first_use_path(node, source),
            _ => None,
        }
    }

    /// AS-002 imported_names:
    ///   `namespace App\X;`        → []
    ///   `use App\X\Y;`             → ["Y"]
    ///   `use X\{A, B};`            → ["A", "B"]
    ///   `use function strlen;`     → ["strlen"]
    ///   `use App\X as Y;`          → ["Y"]
    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if node.kind() != "namespace_use_declaration" {
            return Vec::new();
        }
        let mut out = Vec::new();
        walk_use_clauses(node, source, &mut out, |clause, src, out| {
            push_clause_local_name(clause, src, out)
        });
        out
    }

    fn extract_imported_aliases(&self, node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
        if node.kind() != "namespace_use_declaration" {
            return Vec::new();
        }
        let mut out = Vec::new();
        walk_use_clauses(node, source, &mut out, |clause, src, out| {
            if let Some(pair) = clause_alias_pair(clause, src) {
                out.push(pair);
            }
        });
        out
    }

    /// AS-003 EXTENDS — walks base_clause / class_interface_clause / body trait
    /// use_declaration. Qualified bases stripped to trailing identifier.
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "base_clause" | "class_interface_clause" => {
                    collect_qualified_or_name(&child, source, &mut out);
                }
                "declaration_list" => {
                    let mut dc = child.walk();
                    for member in child.children(&mut dc) {
                        if member.kind() == "use_declaration" {
                            collect_qualified_or_name(&member, source, &mut out);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

// -----------------------------------------------------------------------------
// Callee extractors
// -----------------------------------------------------------------------------

/// AS-018 security canary — tree-sitter-php parses `{$x->m()}` / `${func()}`
/// / `{X::m()}` inside heredoc complex interpolation as REAL call expression
/// nodes. Treating them as call sites poisons the graph (attacker plants
/// fake CALLS by embedding interpolation in a heredoc).
///
/// Walk parents — if any ancestor is `heredoc_body` or `nowdoc_body`, the
/// node lives inside string content. Suppress emission.
fn is_inside_string_body(node: &Node<'_>) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if matches!(n.kind(), "heredoc_body" | "nowdoc_body") {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// member_call / nullsafe_member_call / scoped_call all carry the callee on
/// the `name` field. Receiver lives elsewhere — never returned here.
fn extract_member_call_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if is_inside_string_body(node) {
        return None;
    }
    node.child_by_field_name("name")?
        .utf8_text(source)
        .ok()
        .map(str::to_string)
}

/// Bare `f()` — `function` field is identifier / qualified_name.
fn extract_function_call_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if is_inside_string_body(node) {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    trailing_segment(&func, source)
}

/// `new X(args)` — type is the first named child (may be `name` or
/// `qualified_name`). The `new` keyword is anonymous so named_children skips it.
fn extract_object_creation_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if is_inside_string_body(node) {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "name" | "qualified_name" | "variable_name") {
            return trailing_segment(&child, source);
        }
    }
    None
}

/// Strip backslash-separated namespace prefix to trailing identifier.
fn trailing_segment(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    text.rsplit('\\').next().map(str::to_string)
}

// -----------------------------------------------------------------------------
// Use-clause walking (AS-002)
// -----------------------------------------------------------------------------

fn walk_use_clauses<T>(
    node: &Node<'_>,
    source: &[u8],
    out: &mut Vec<T>,
    handle: impl Fn(&Node<'_>, &[u8], &mut Vec<T>),
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => handle(&child, source, out),
            "namespace_use_group" => {
                let mut gc = child.walk();
                for g in child.children(&mut gc) {
                    if g.kind() == "namespace_use_clause" {
                        handle(&g, source, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn first_use_path(node: &Node<'_>, source: &[u8]) -> Option<String> {
    // tree-sitter-php emits two shapes:
    //   - `use X;` / `use X as Y;` → namespace_use_declaration → namespace_use_clause(qualified_name|name, ...)
    //   - `use X\Y\{A, B};`        → namespace_use_declaration → namespace_name (prefix), namespace_use_group {...}
    // The group case puts the prefix as a SIBLING of namespace_use_group (NOT inside it).
    let mut cursor = node.walk();
    let mut prefix: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => return clause_path(&child, source),
            "namespace_name" | "qualified_name" | "name" => {
                if prefix.is_none() {
                    prefix = child.utf8_text(source).ok().map(str::to_string);
                }
            }
            "namespace_use_group" => {
                // Group form — return the prefix if we captured one.
                return prefix;
            }
            _ => {}
        }
    }
    prefix
}

fn clause_path(clause: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = clause.walk();
    for child in clause.named_children(&mut cursor) {
        if matches!(child.kind(), "qualified_name" | "name" | "namespace_name") {
            return child.utf8_text(source).ok().map(str::to_string);
        }
    }
    None
}

/// Local-bound name for a `namespace_use_clause`:
/// `use X;` → "X"; `use X\Y;` → "Y"; `use X as Y;` → "Y".
fn push_clause_local_name(clause: &Node<'_>, source: &[u8], out: &mut Vec<String>) {
    let (path_text, alias) = clause_path_and_alias(clause, source);
    if let Some(a) = alias {
        out.push(a);
        return;
    }
    if let Some(p) = path_text {
        if let Some(last) = p.rsplit('\\').next() {
            if !last.is_empty() {
                out.push(last.to_string());
            }
        }
    }
}

fn clause_alias_pair(clause: &Node<'_>, source: &[u8]) -> Option<(String, String)> {
    let (Some(p), Some(a)) = clause_path_and_alias(clause, source) else {
        return None;
    };
    let original = p.rsplit('\\').next().unwrap_or(&p).to_string();
    Some((a, original))
}

fn clause_path_and_alias(clause: &Node<'_>, source: &[u8]) -> (Option<String>, Option<String>) {
    // tree-sitter-php emits `use X as Y` as:
    //   namespace_use_clause(qualified_name|name "X", `as`, name "Y")
    // The alias is a bare named `name` AFTER the path-like child. There is no
    // `use_as_clause` wrapper at this layer (that node exists in the grammar
    // but applies to trait-use `insteadof`/`as` inside class bodies).
    let mut cursor = clause.walk();
    let mut path_text: Option<String> = None;
    let mut alias: Option<String> = None;
    for child in clause.named_children(&mut cursor) {
        match child.kind() {
            "qualified_name" | "namespace_name" if path_text.is_none() => {
                path_text = child.utf8_text(source).ok().map(str::to_string);
            }
            "name" => {
                // First `name` = path (e.g. `use X;` simple form).
                // Subsequent `name` = alias (after the `as` keyword).
                if path_text.is_none() {
                    path_text = child.utf8_text(source).ok().map(str::to_string);
                } else if alias.is_none() {
                    alias = child.utf8_text(source).ok().map(str::to_string);
                }
            }
            _ => {}
        }
    }
    (path_text, alias)
}

// -----------------------------------------------------------------------------
// EXTENDS helpers (AS-003)
// -----------------------------------------------------------------------------

/// Collect trailing identifier names from a base_clause / class_interface_clause /
/// use_declaration subtree. Strips namespace prefix.
fn collect_qualified_or_name(node: &Node<'_>, source: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "name" => {
                if let Ok(text) = child.utf8_text(source) {
                    if !text.is_empty() {
                        out.push(text.to_string());
                    }
                }
            }
            "qualified_name" => {
                if let Ok(text) = child.utf8_text(source) {
                    if let Some(last) = text.rsplit('\\').next() {
                        if !last.is_empty() {
                            out.push(last.to_string());
                        }
                    }
                }
            }
            _ => collect_qualified_or_name(&child, source, out),
        }
    }
}

// -----------------------------------------------------------------------------
// AS-004 — DI annotation ref emitter
// -----------------------------------------------------------------------------

/// One ParsedReference per DI-annotated property. Wired on
/// `property_declaration` (not on `attribute`) so multi-attribute properties
/// dedupe to ONE ref. Non-DI attrs (#[ORM\Column]) route via extract_attributes
/// to Decorator(...) — never reach this emitter.
fn extract_annotated_property_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    if !property_has_di_attribute(node, source) {
        return;
    }
    let Some(target_name) = property_type_name(node, source) else {
        return;
    };
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name,
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::AnnotatedFieldType,
    });
}

fn property_has_di_attribute(prop: &Node<'_>, source: &[u8]) -> bool {
    // tree-sitter-php structure: property_declaration → attribute_list →
    // attribute_group(s) → attribute → name (or qualified_name for namespaced).
    let mut cursor = prop.walk();
    for child in prop.children(&mut cursor) {
        if child.kind() != "attribute_list" {
            continue;
        }
        let mut alc = child.walk();
        for grp in child.children(&mut alc) {
            if grp.kind() != "attribute_group" {
                continue;
            }
            let mut gc = grp.walk();
            for attr in grp.named_children(&mut gc) {
                if attr.kind() != "attribute" {
                    continue;
                }
                // `#[Required]` → first named child is `name`.
                // `#[ORM\Column]` → first named child is `qualified_name` —
                // Lang-C7 carve-out: namespaced attrs are NEVER DI markers.
                let mut ac = attr.walk();
                for an in attr.named_children(&mut ac) {
                    match an.kind() {
                        "name" => {
                            if let Ok(text) = an.utf8_text(source) {
                                if PHP_DI_ATTRIBUTES.contains(&text) {
                                    return true;
                                }
                            }
                            break;
                        }
                        "qualified_name" => break,
                        _ => {}
                    }
                }
            }
        }
    }
    false
}

fn property_type_name(prop: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = prop.walk();
    for child in prop.named_children(&mut cursor) {
        if let Some(name) = type_to_name(&child, source) {
            return Some(name);
        }
    }
    None
}

/// Unwrap PHP type nodes to a single trailing identifier.
/// Covers: primitive_type / name / qualified_name / named_type / optional_type.
fn type_to_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "primitive_type" | "name" => node.utf8_text(source).ok().map(str::to_string),
        "qualified_name" => {
            let text = node.utf8_text(source).ok()?;
            Some(text.rsplit('\\').next().unwrap_or(text).to_string())
        }
        "named_type" | "optional_type" => {
            let mut c = node.walk();
            for inner in node.named_children(&mut c) {
                if let Some(n) = type_to_name(&inner, source) {
                    return Some(n);
                }
            }
            None
        }
        _ => None,
    }
}
