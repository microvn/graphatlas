//! Tools S-001 cluster B — extract call-site records from a tree.
//!
//! For each call-node encountered during walk, emit `(enclosing_symbol,
//! callee_name, call_site_line)`. The indexer uses these to build CALLS
//! edges. Within-file resolution + cross-file resolution are cluster-C
//! concerns; this module only surfaces the raw call sites.
//!
//! Callee name extraction ported from `rust-poc/src/main.rs:418-511`.

use crate::LanguageSpec;
use ga_core::{Lang, Result};
use tree_sitter::{Node, Parser};

/// Engine-side callee dispatcher (S-005a D4). Per-lang `callee_extractors()`
/// table takes precedence; falls back to `extract_standard_callee` for
/// generic `call_expression` / `call` handling. Replaces the pre-D4
/// `extract_call_name` which inlined per-lang branches (decorator, new,
/// jsx, macro_invocation) inside the engine.
pub(crate) fn dispatch_callee(
    spec: &dyn LanguageSpec,
    node: &Node<'_>,
    source: &[u8],
) -> Option<String> {
    let kind = node.kind();
    for (registered_kind, extractor) in spec.callee_extractors() {
        if *registered_kind == kind {
            return extractor(node, source);
        }
    }
    extract_standard_callee(node, source)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCall {
    /// Name of the symbol that contains this call site — `None` for
    /// module-level (top-of-file) calls.
    pub enclosing_symbol: Option<String>,
    /// Callee short name (no module prefix, no receiver).
    pub callee_name: String,
    /// 1-based line number of the call site.
    pub call_site_line: u32,
}

/// Parse `source` for `lang` and emit every call site as a [`ParsedCall`].
pub fn extract_calls(lang: Lang, source: &[u8]) -> Result<Vec<ParsedCall>> {
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
    Ok(extract_calls_from_tree(spec, tree.root_node(), source))
}

/// Variant of [`extract_calls`] that reuses an already-parsed tree. The
/// indexer parses once per file then dispatches all 4 extractors on the same
/// `Tree` to avoid the ~5× redundant tree-sitter parse cost.
pub fn extract_calls_from_tree(
    spec: &dyn LanguageSpec,
    root: Node<'_>,
    source: &[u8],
) -> Vec<ParsedCall> {
    let mut out = Vec::new();
    walk(root, source, spec, None, &mut out);
    out
}

fn walk(
    node: Node<'_>,
    source: &[u8],
    spec: &dyn LanguageSpec,
    enclosing: Option<String>,
    out: &mut Vec<ParsedCall>,
) {
    let kind = node.kind();

    // If this is a symbol-defining node (function / class / method), push its
    // name as the new enclosing context for calls inside it.
    let new_enclosing = if spec.is_symbol_node(kind) {
        crate::name_from_node(&node, source).or(enclosing.clone())
    } else {
        enclosing.clone()
    };

    // If this is a call-node, record it.
    if spec.is_call_node(kind) {
        if let Some(callee) = dispatch_callee(spec, &node, source) {
            out.push(ParsedCall {
                enclosing_symbol: enclosing.clone(),
                callee_name: callee,
                call_site_line: (node.start_position().row as u32) + 1,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, spec, new_enclosing.clone(), out);
    }
}

/// Standard `call_expression` / `call` callee handling — fallback when no
/// per-lang `callee_extractors()` table entry matches. Ported from
/// `rust-poc/src/main.rs:418-511`. Handles all v1 langs uniformly via
/// `match func.kind()` (no `match Lang::*`).
///
/// S-005a D4 — extracted from pre-D4 `extract_call_name` after the four
/// per-kind branches (decorator, new_expression, jsx, macro_invocation)
/// were migrated into `langs/{py,ts,js,rs}.rs` callee_extractors tables.
pub(crate) fn extract_standard_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let func = node
        .child_by_field_name("function")
        .or_else(|| node.child(0))?;
    match func.kind() {
        "identifier" => func.utf8_text(source).ok().map(|s| s.to_string()),
        "member_expression" | "attribute" | "selector_expression" | "field_expression" => {
            let prop = func
                .child_by_field_name("property")
                .or_else(|| func.child_by_field_name("attribute"))
                .or_else(|| func.child_by_field_name("field"));
            if let Some(p) = prop {
                return p.utf8_text(source).ok().map(|s| s.to_string());
            }
            let text = func.utf8_text(source).ok()?;
            text.split('.').next_back().map(|s| s.to_string())
        }
        "scoped_identifier" | "qualified_name" => {
            let text = func.utf8_text(source).ok()?;
            text.rsplit("::")
                .next()
                .or_else(|| text.rsplit('.').next())
                .map(|s| s.to_string())
        }
        _ => {
            let text = func.utf8_text(source).ok()?;
            text.rsplit('.')
                .next()
                .or_else(|| text.rsplit("::").next())
                .map(|s| s.to_string())
        }
    }
}
