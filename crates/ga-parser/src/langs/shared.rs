//! Extraction helpers shared across languages.
//! Ported from rust-poc/src/main.rs:382-417.

use crate::references::is_clean_ident;
use crate::{ParsedReference, RefKind};
use tree_sitter::Node;

/// PR5b — extract return-type text from a function/method node by field name.
/// Strips a leading `->`, `=>`, or `:` arrow plus any wrapping
/// `type_annotation`/`type_clause` punctuation. Returns trimmed raw type
/// text, or `None` if the field is absent.
pub fn extract_return_type_by_field(node: &Node<'_>, source: &[u8], field: &str) -> Option<String> {
    let rt_node = node.child_by_field_name(field)?;
    // TypeScript wraps the return type in a `type_annotation` whose first
    // child is the `:` token. Unwrap one level when the field carries
    // punctuation. Same idea for languages whose grammar surfaces the
    // arrow as part of the field's text.
    let raw = rt_node.utf8_text(source).ok()?.trim();
    let stripped = raw
        .strip_prefix("->")
        .or_else(|| raw.strip_prefix("=>"))
        .unwrap_or(raw)
        .trim()
        .strip_prefix(':')
        .unwrap_or_else(|| {
            raw.strip_prefix("->")
                .or_else(|| raw.strip_prefix("=>"))
                .unwrap_or(raw)
                .trim()
        })
        .trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

/// PR5c2b — generic param extraction helper for langs whose parameter
/// container yields children with predictable field names. Walks
/// `container_field` of `node`, then iterates each named child trying:
/// 1. Field "name" → fallback first `identifier` named child
/// 2. Field "type" → strip leading `:` / `->` punctuation
/// 3. Field "value" → fallback field "default"
///
/// Each lang spec's `extract_params` thin-wraps this with the right
/// `container_field` name. Special-purpose params (Rust `self_parameter`,
/// Go grouped-decl, Ruby `optional_parameter`) need per-lang impls.
pub fn extract_params_by_container(
    node: &Node<'_>,
    source: &[u8],
    container_field: &str,
) -> Vec<crate::ParsedParam> {
    let plist = match node.child_by_field_name(container_field) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut cursor = plist.walk();
    for child in plist.named_children(&mut cursor) {
        let kind = child.kind();
        // `identifier` (untyped) → name only
        if kind == "identifier" {
            if let Ok(text) = child.utf8_text(source) {
                let t = text.trim();
                if !t.is_empty() {
                    out.push(crate::ParsedParam {
                        name: t.to_string(),
                        type_: String::new(),
                        default_value: String::new(),
                    });
                }
            }
            continue;
        }
        // Skip non-parameter kinds (commas, parens). Heuristic: kind must
        // include "parameter" OR be one of the structural kinds we
        // recognize.
        if !kind.contains("parameter") && kind != "variable_declaration" {
            continue;
        }
        // Name: try field "name", then "pattern", then first identifier
        // descendant.
        let name = child
            .child_by_field_name("name")
            .or_else(|| child.child_by_field_name("pattern"))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim().to_string())
            .or_else(|| {
                let mut cc = child.walk();
                let found = child
                    .named_children(&mut cc)
                    .find(|c| c.kind() == "identifier" || c.kind() == "simple_identifier")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.trim().to_string());
                found
            })
            .unwrap_or_default();
        // Type: try "type", strip leading `:` from type_annotation wrapper.
        let ty = child
            .child_by_field_name("type")
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| {
                let t = s.trim();
                t.strip_prefix(':').unwrap_or(t).trim().to_string()
            })
            .unwrap_or_default();
        let default = child
            .child_by_field_name("value")
            .or_else(|| child.child_by_field_name("default"))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !name.is_empty() {
            out.push(crate::ParsedParam {
                name,
                type_: ty,
                default_value: default,
            });
        }
    }
    out
}

/// PR5c2b — generic modifier extraction. Looks for a sibling `modifiers`
/// node and collects each non-annotation child's text. Handles Java /
/// Kotlin / C# AST shape where modifiers cluster in a single block.
pub fn extract_modifiers_block(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    let mut mods = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "modifiers" {
            continue;
        }
        // Iterate ALL children (including anonymous keyword tokens like
        // `public` / `static` / `final` which tree-sitter often does NOT
        // expose as named nodes). Filter annotations (named) which become
        // DECORATES edges in PR8.
        let mut cc = child.walk();
        for m in child.children(&mut cc) {
            let mk = m.kind();
            if mk.contains("annotation") || mk == "decorator" {
                continue;
            }
            if let Ok(t) = m.utf8_text(source) {
                let t = t.trim();
                if !t.is_empty() {
                    mods.push(t.to_string());
                }
            }
        }
        break;
    }
    mods
}

/// JS/TS `new_expression` callee extraction — `new Foo()` → "Foo",
/// `new pkg.Bar()` → "Bar". S-005a D4 — migrated from calls.rs.
pub fn extract_new_expression_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let constructor = node
        .child_by_field_name("constructor")
        .or_else(|| node.child(1))?;
    match constructor.kind() {
        "identifier" | "type_identifier" => {
            constructor.utf8_text(source).ok().map(|s| s.to_string())
        }
        "member_expression" => {
            let prop = constructor.child_by_field_name("property")?;
            prop.utf8_text(source).ok().map(|s| s.to_string())
        }
        _ => {
            let text = constructor.utf8_text(source).ok()?;
            text.split('.').next_back().map(|s| s.to_string())
        }
    }
}

/// JSX uppercase element callee — `<Foo />` and `<Foo>...</Foo>` both
/// surface as call to `Foo`. Lowercase tags (`<div>`) are HTML elements,
/// not function/component refs — explicitly NOT recorded.
/// S-005a D4 — migrated from calls.rs (JSX self_closing + opening branches).
pub fn extract_jsx_element_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name").or_else(|| node.child(1))?;
    let text = name_node.utf8_text(source).ok()?;
    if text.starts_with(|c: char| c.is_uppercase()) {
        text.split('.').next_back().map(|s| s.to_string())
    } else {
        None
    }
}

/// JS/TS shorthand-property reference emitter — `{ handleClick }` is
/// equivalent to `{ handleClick: handleClick }` and the bare identifier
/// names a function reference.
///
/// S-005a D3 — migrated from references.rs engine `match kind` with
/// `if matches!(lang, Lang::TypeScript | Lang::JavaScript)` guard. The
/// `shorthand_property_identifier` node kind is JS/TS-grammar-only so the
/// guard was defense-in-depth; lookup-based dispatch makes it intrinsic.
pub fn emit_shorthand_property_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let Ok(name) = node.utf8_text(source) else {
        return;
    };
    if !is_clean_ident(name) {
        return;
    }
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name: name.to_string(),
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::Shorthand,
    });
}

/// Walk the children of an import-shaped node and return the first string /
/// dotted_name literal we find (depth ≤ 2). Matches the behavior of
/// rust-poc's `extract_import_path` for Python / TS / JS / Go.
pub fn extract_import_path_default(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(s) = literal_or_dotted(&child, source) {
            return Some(s);
        }
        let mut inner = child.walk();
        for grandchild in child.children(&mut inner) {
            if let Some(s) = literal_or_dotted(&grandchild, source) {
                return Some(s);
            }
        }
    }
    None
}

fn literal_or_dotted(node: &Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "string" | "string_literal" | "interpreted_string_literal" | "string_content" => {
            let text = node.utf8_text(source).ok()?;
            let trimmed = text.trim_matches(|c| c == '"' || c == '\'' || c == '`');
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        "dotted_name" | "module_name" => Some(node.utf8_text(source).ok()?.to_string()),
        _ => None,
    }
}
