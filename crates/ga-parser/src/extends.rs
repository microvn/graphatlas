//! Extract `(class_name, base_name)` pairs so the indexer can write EXTENDS
//! edges. Mirrors the shape of [`crate::calls::extract_calls`] and
//! [`crate::imports::extract_imports`] — walk the tree, emit raw records,
//! let the indexer resolve cross-file links in a second pass.
//!
//! Powers auto-GT H1 (polymorphism): H1 needs to enumerate class
//! inheritance to find method overrides. Without EXTENDS populated, H1
//! has to re-parse files — wasteful and duplicated. Putting EXTENDS in
//! the graph once makes H1 a pure graph query.

use crate::LanguageSpec;
use ga_core::{Lang, Result};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedExtends {
    /// Name of the deriving class.
    pub class_name: String,
    /// 1-based line of the class definition (for symbol id matching later).
    pub class_line: u32,
    /// Short base name (module prefix stripped per lang spec).
    pub base_name: String,
}

pub fn extract_extends(lang: Lang, source: &[u8]) -> Result<Vec<ParsedExtends>> {
    let pool = crate::ParserPool::new();
    let Some(spec) = pool.spec_for(lang) else {
        return Err(ga_core::Error::Other(anyhow::anyhow!(
            "no LanguageSpec for {lang:?}"
        )));
    };
    let mut parser = Parser::new();
    parser
        .set_language(&spec.tree_sitter_lang())
        .map_err(|e| ga_core::Error::ParseError {
            file: "<source>".into(),
            lang: lang.as_str().into(),
            err: format!("set_language: {e}"),
        })?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ga_core::Error::ParseError {
            file: "<source>".into(),
            lang: lang.as_str().into(),
            err: "no tree".into(),
        })?;
    Ok(extract_extends_from_tree(spec, tree.root_node(), source))
}

/// Variant of [`extract_extends`] that reuses an already-parsed tree.
pub fn extract_extends_from_tree(
    spec: &dyn LanguageSpec,
    root: Node<'_>,
    source: &[u8],
) -> Vec<ParsedExtends> {
    let mut out = Vec::new();
    walk(root, source, spec, &mut out);
    out
}

fn walk(node: Node<'_>, source: &[u8], spec: &dyn LanguageSpec, out: &mut Vec<ParsedExtends>) {
    let kind = node.kind();
    if spec.is_extends_node(kind) {
        // Rust `impl_item` has no `name` field — its identity comes from the
        // `type` child (the struct/enum being implemented for). Python/TS
        // class_definition / class_declaration expose `name` directly, so
        // name_from_node works for them. Try the standard path first, fall
        // back to impl-type extraction for Rust.
        let class_name =
            crate::name_from_node(&node, source).or_else(|| extract_impl_type_name(&node, source));
        if let Some(class_name) = class_name {
            let class_line = (node.start_position().row as u32) + 1;
            for base in spec.extract_bases(&node, source) {
                // class_name sanity: identifier-only to match the allowlist
                // used at storage + query time (Tools-C9-d).
                if !is_ident_like(&class_name) || !is_ident_like(&base) {
                    continue;
                }
                out.push(ParsedExtends {
                    class_name: class_name.clone(),
                    class_line,
                    base_name: base,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, spec, out);
    }
}

/// Rust-specific: extract the implementing type from `impl Trait for Struct`
/// (or `impl Struct`) — the `type` field. Strips generics.
fn extract_impl_type_name(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "impl_item" {
        return None;
    }
    let type_node = node.child_by_field_name("type")?;
    let text = type_node.utf8_text(source).ok()?;
    let name = text.split('<').next().unwrap_or(text).trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn is_ident_like(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 512
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
}
