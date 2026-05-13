//! Foundation-C15 — extract value-reference sites: function symbols held by
//! value rather than called. Covers dispatch maps, callback arrays, and
//! shorthand property references.
//!
//! Per-lang structural patterns (ported from legacy
//! `rust-poc/src/main.rs::extract_value_references`, line 1261):
//!
//!   JS / TS / TSX
//!     - `pair` node (object key-value): `{ '/api': handleUsers }`
//!     - `array` element identifiers: `[onStart, onDone]`
//!     - `shorthand_property_identifier` inside object: `{ handleClick }`
//!
//!   Python
//!     - `pair` in `dictionary`: `{'key': fn_ref}`
//!     - `list` / `tuple` element identifiers
//!
//!   Go, Rust
//!     - DEFERRED per Foundation-C15. struct-field fn assignment + `fn`
//!       pointer passing are per-lang specific and land in v1.x.
//!
//! Resolution happens in a second pass at the indexer layer (same way
//! `extract_calls` emits raw records + indexer resolves). This module
//! only surfaces candidate identifiers in the four structural positions;
//! stopword filtering drops obvious non-functions (keywords, constants).

use crate::LanguageSpec;
use ga_core::{Lang, Result};
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedReference {
    /// Enclosing function that holds the reference. `None` = module scope.
    pub enclosing_symbol: Option<String>,
    /// Identifier text — the function name being referenced.
    pub target_name: String,
    /// 1-based line of the identifier.
    pub ref_site_line: u32,
    /// Structural position distinguishing the context.
    pub ref_kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// Object pair value: `{ key: fn }`
    MapValue,
    /// Array / list element: `[fn]`
    ArrayElem,
    /// Object shorthand: `{ fn }`
    Shorthand,
    /// Go struct-field function assignment: `Handler{OnClick: handleClick}`
    /// (infra:S-001 AS-001).
    StructFieldFn,
    /// Rust fn pointer passed as function argument: `register(on_click)`
    /// (infra:S-001 AS-002).
    FnPointerArg,
    /// v1.1-M4 S-001c (AS-004) — type referenced by an annotated field.
    /// `@Autowired private UserRepository repo;` → REFERENCES edge to
    /// `UserRepository` so `ga_impact {symbol: UserRepository}` surfaces
    /// the consuming class. Generalizes to Kotlin `@Inject`, C# `[Inject]`,
    /// and any DI-style annotated field pattern.
    AnnotatedFieldType,
    /// 2026-04-28 — type identifier appearing in a type position.
    /// Examples:
    ///   - Go: `var x Foo`, `Foo{...}`, `&Foo`, `[]Foo`, `func(Foo)`.
    ///   - Rust: `let x: Foo`, `-> Foo`, `Vec<Foo>`, `Foo::new()` (subset).
    ///   - TS/JS: `let x: Foo`, `Foo<G>`, `function f(x: Foo)`.
    /// Driver: M3 dead_code FP audit — engine flagged Go/Rust/TS types
    /// dead because type-position uses weren't emitted as REFERENCES
    /// edges, while raw-text scan in Hd-ast GT did see them.
    /// Emitted via per-lang emitter on `type_identifier` node kind.
    TypePosition,
}

impl RefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MapValue => "map_value",
            Self::ArrayElem => "array_elem",
            Self::Shorthand => "shorthand",
            Self::StructFieldFn => "struct_field_fn",
            Self::FnPointerArg => "fn_pointer_arg",
            Self::AnnotatedFieldType => "annotated_field_type",
            Self::TypePosition => "type_position",
        }
    }
}

pub fn extract_references(lang: Lang, source: &[u8]) -> Result<Vec<ParsedReference>> {
    let pool = crate::ParserPool::new();
    let Some(spec) = pool.spec_for(lang) else {
        return Err(ga_core::Error::Other(anyhow::anyhow!(
            "no LanguageSpec for {lang:?}"
        )));
    };
    // infra:S-001 (v1.1-M0) — Go/Rust StructFieldFn + FnPointerArg emission
    // lifted. Other Go/Rust ref_kind variants (map value, slice elem,
    // positional composite literal) intentionally still deferred.
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
    Ok(extract_references_from_tree(
        spec,
        lang,
        tree.root_node(),
        source,
    ))
}

/// Variant of [`extract_references`] that reuses an already-parsed tree.
pub fn extract_references_from_tree(
    spec: &dyn LanguageSpec,
    lang: Lang,
    root: Node<'_>,
    source: &[u8],
) -> Vec<ParsedReference> {
    let mut out = Vec::new();
    walk(root, source, spec, lang, None, &mut out);
    out
}

fn walk(
    node: Node<'_>,
    source: &[u8],
    spec: &dyn LanguageSpec,
    lang: Lang,
    enclosing: Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let kind = node.kind();

    // Track enclosing function for all reference sites inside it.
    let new_enclosing = if spec.is_symbol_node(kind) {
        crate::name_from_node(&node, source).or(enclosing.clone())
    } else {
        enclosing.clone()
    };

    // S-005a D3 — per-lang emitter table dispatch FIRST. Replaces the
    // pre-D3 `if matches!(lang, Lang::Go)` / `Lang::Rust` / `Lang::TS|JS`
    // branches that previously lived inside the engine `match kind` block.
    // Per `engine_no_lang_match.rs` (D6) regression guard, the engine MUST
    // NOT contain `match Lang::*` patterns.
    let mut handled_by_lang = false;
    for (registered_kind, emitter) in spec.ref_emitters() {
        if *registered_kind == kind {
            emitter(&node, source, &enclosing, out);
            handled_by_lang = true;
            break;
        }
    }

    // Cross-lang structural emitters — `pair`, `array`, `list` are universal
    // AST shapes (JS/TS/Python all emit them with the same semantics).
    // Engine handles them centrally to avoid duplicating the same code in
    // 3+ lang impls. Per-lang ref_emitters take precedence above.
    if !handled_by_lang {
        match kind {
            "pair" => emit_pair_value(&node, source, &enclosing, lang, out),
            "array" | "list" => emit_array_elements(&node, source, &enclosing, out),
            _ => {}
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, spec, lang, new_enclosing.clone(), out);
    }
}

fn emit_pair_value(
    pair: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    lang: Lang,
    out: &mut Vec<ParsedReference>,
) {
    // tree-sitter-{js,ts,python}: `pair` has `key` field + `value` field.
    let Some(value_node) = pair.child_by_field_name("value") else {
        // Python dictionary pairs don't have explicit `value` field —
        // structure is [key, :, value]. Take last named child.
        let last = last_named_child(pair);
        let Some(value_node) = last else { return };
        emit_if_identifier_ref(&value_node, source, enclosing, RefKind::MapValue, out, lang);
        return;
    };
    emit_if_identifier_ref(&value_node, source, enclosing, RefKind::MapValue, out, lang);
}

/// Last named child of a node — re-used by per-lang ref emitters in
/// `langs/{go,rs}.rs` after S-005a D3 migration.
pub(crate) fn last_named_child<'t>(node: &Node<'t>) -> Option<Node<'t>> {
    let count = node.named_child_count();
    if count == 0 {
        return None;
    }
    node.named_child(count - 1)
}

fn emit_array_elements(
    arr: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let mut cursor = arr.walk();
    for elem in arr.children(&mut cursor) {
        if elem.kind() == "identifier" {
            if let Ok(name) = elem.utf8_text(source) {
                if is_clean_ident(name) {
                    out.push(ParsedReference {
                        enclosing_symbol: enclosing.clone(),
                        target_name: name.to_string(),
                        ref_site_line: (elem.start_position().row as u32) + 1,
                        ref_kind: RefKind::ArrayElem,
                    });
                }
            }
        }
    }
}

// S-005a D3 — emit_go_keyed_element + emit_rust_call_arg_identifiers
// migrated to crates/ga-parser/src/langs/{go,rs}.rs as per-lang
// `ref_emitters()` table entries. Engine no longer dispatches on
// `match Lang::*`; see `walk()` above.

fn emit_if_identifier_ref(
    value_node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    ref_kind: RefKind,
    out: &mut Vec<ParsedReference>,
    _lang: Lang,
) {
    if value_node.kind() != "identifier" {
        return;
    }
    let Ok(name) = value_node.utf8_text(source) else {
        return;
    };
    if !is_clean_ident(name) {
        return;
    }
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name: name.to_string(),
        ref_site_line: (value_node.start_position().row as u32) + 1,
        ref_kind,
    });
}

/// Stopword + shape filter ported from rust-poc/src/main.rs::emit_reference_if_known (line 1319-1326).
/// Rejects keywords, 1-char names, and ALL_CAPS constants (uppercase + underscores only).
/// `pub(crate)` so per-lang ref emitters in `langs/{go,rs,ts,js}.rs` can apply
/// the same filter after S-005a D3 migration.
/// Like `is_clean_ident` but without the ALL_CAPS filter. Used for
/// TypePosition emitters where all-caps names are valid type names
/// (e.g. `DFA`, `BE`, `NFA`, `IO`) — `type_identifier` nodes in
/// tree-sitter grammars are never constants, so the ALL_CAPS heuristic
/// is wrong in that context.
pub(crate) fn is_type_ident(name: &str) -> bool {
    if name.is_empty() || name.len() <= 1 || name.len() > 512 {
        return false;
    }
    if matches!(
        name,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "None"
            | "True"
            | "False"
            | "self"
            | "this"
            | "cls"
            | "super"
    ) {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

pub(crate) fn is_clean_ident(name: &str) -> bool {
    if name.is_empty() || name.len() <= 1 || name.len() > 512 {
        return false;
    }
    if matches!(
        name,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "None"
            | "True"
            | "False"
            | "self"
            | "this"
            | "cls"
            | "super"
    ) {
        return false;
    }
    // ALL_CAPS constants — heuristic: all chars uppercase or underscore.
    if name.chars().all(|c| c.is_uppercase() || c == '_') {
        return false;
    }
    // Identifier allowlist (Tools-C9-d compatible).
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}
