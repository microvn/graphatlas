//! Tools S-003 — extract import-site records from a source tree.
//!
//! Mirrors `crate::calls::extract_calls`: walk the tree, record every node
//! matching `LanguageSpec::is_import_node`. Cluster B additions:
//!   - `imported_names` populated best-effort per lang
//!   - TS/JS `export … from '…'` surfaces as `is_re_export = true`

use crate::LanguageSpec;
use ga_core::{Lang, Result};
use tree_sitter::{Node, Parser};

// ─────────────────────────────────────────────────────────────────────────
// v1.1-M4 S-001c (Lang-C6 migration) — engine purity for imports.rs.
//
// Pre-S-001c this file branched on `match lang { Python | TS|JS, _ => Vec::new() }`
// for `imported_names`, `imported_aliases`, and the `export_statement`
// re-export detection. That kept new langs (Java/Kotlin/CSharp/Ruby) silent
// in those slots — exactly the carve-out tracked in the S-005a checklist
// (destination: languages:S-001).
//
// Post-S-001c the engine reads three trait methods on `LanguageSpec`:
//   - extract_imported_names
//   - extract_imported_aliases
//   - extract_re_export
// Each per-lang impl in `langs/<lang>.rs` decides what to populate. The
// existing helper functions stay in this file as `pub(crate)` so per-lang
// impls reuse them without duplication; the *dispatch* moved to the trait.
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedImport {
    /// Raw import target as written in source.
    /// - Python: dotted module path, e.g. `utils.format`
    /// - TS/JS/Go: literal string path, e.g. `./utils/format` / `github.com/foo/bar`
    /// - Rust: first token of the `use_declaration`
    pub target_path: String,
    /// Symbol names pulled in by this import site. Best-effort — covers the
    /// common forms per language (named imports, `from X import a, b`, etc.).
    /// For aliased forms (`import X as Y`, `import { X as Y } from …`) this
    /// contains the LOCAL name (`Y`) — the name used at call sites.
    pub imported_names: Vec<String>,
    /// infra:S-002 AS-005 — alias pairs for cross-file resolution.
    /// `(local, original)` only when an alias is present. Example:
    /// `from mod.b import Foo as F` → `[("F", "Foo")]`. Empty for
    /// non-aliased imports. The indexer consults this to resolve
    /// `F()` at the call site to `mod/b.py::Foo` instead of
    /// `__external__::F`.
    pub imported_aliases: Vec<(String, String)>,
    /// 1-based line of the import statement.
    pub import_line: u32,
    /// Cluster B — TS/JS only: `true` for `export * from '…'` and
    /// `export { X } from '…'`. Indexer propagates to the IMPORTS edge so
    /// AS-006 responses can mark re-exports.
    pub is_re_export: bool,
    /// v1.4 S-002 / AS-015..017 — TS-only: names imported with the
    /// `type` modifier (`import type { Foo }` whole-statement OR
    /// `import { Foo, type Bar }` per-name). Indexer cross-references
    /// this list when emitting IMPORTS_NAMED rows to populate the
    /// `is_type_only` column. For aliased imports (`import { X as Y }`)
    /// the LOCAL name (Y) is what shows up here. For re-export forms
    /// (`export type { Foo } from 'mod'`), Foo appears in this list with
    /// `is_re_export = true` per AS-017. Empty for non-TS langs (no
    /// equivalent type-only distinction at source level).
    pub type_only_names: Vec<String>,
}

pub fn extract_imports(lang: Lang, source: &[u8]) -> Result<Vec<ParsedImport>> {
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
    Ok(extract_imports_from_tree(spec, tree.root_node(), source))
}

/// Variant of [`extract_imports`] that reuses an already-parsed tree.
pub fn extract_imports_from_tree(
    spec: &dyn LanguageSpec,
    root: Node<'_>,
    source: &[u8],
) -> Vec<ParsedImport> {
    let mut out = Vec::new();
    walk(root, source, spec, &mut out);
    out
}

fn walk(node: Node<'_>, source: &[u8], spec: &dyn LanguageSpec, out: &mut Vec<ParsedImport>) {
    let kind = node.kind();
    if spec.is_import_node(kind) {
        if let Some(target) = spec.extract_import_path(&node, source) {
            out.push(ParsedImport {
                target_path: target,
                imported_names: spec.extract_imported_names(&node, source),
                imported_aliases: spec.extract_imported_aliases(&node, source),
                import_line: (node.start_position().row as u32) + 1,
                is_re_export: false,
                type_only_names: spec.extract_type_only_names(&node, source),
            });
        }
    }

    // Re-export detection — TS/JS today, but the trait method is
    // language-agnostic (langs without re-export semantics return None).
    if let Some((target, names)) = spec.extract_re_export(&node, source) {
        out.push(ParsedImport {
            target_path: target,
            imported_names: names,
            imported_aliases: Vec::new(),
            import_line: (node.start_position().row as u32) + 1,
            is_re_export: true,
            type_only_names: spec.extract_type_only_names(&node, source),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, spec, out);
    }
}

// --- shared helpers used by per-lang LanguageSpec impls --------------------
// Each helper extracts a specific lang's imported_names / aliases / export
// semantics. Exposed `pub(crate)` so `langs/<lang>.rs` calls them via
// thin trait-method delegates rather than duplicating the parsing logic.

pub(crate) fn python_imported_aliases(node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "aliased_import" | "dotted_as_name" => {
                // tree-sitter-python gives `name` and `alias` field names.
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(|s| s.rsplit('.').next().unwrap_or(s).to_string());
                let alias = child
                    .child_by_field_name("alias")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(str::to_string);
                if let (Some(original), Some(local)) = (name, alias) {
                    if local != original {
                        out.push((local, original));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn ts_js_imported_aliases(node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_ts_aliases(node, source, &mut out);
    out
}

fn collect_ts_aliases(node: &Node<'_>, source: &[u8], out: &mut Vec<(String, String)>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_specifier" | "export_specifier" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(str::to_string);
                let alias = child
                    .child_by_field_name("alias")
                    .and_then(|n| n.utf8_text(source).ok())
                    .map(str::to_string);
                if let (Some(original), Some(local)) = (name, alias) {
                    if local != original {
                        out.push((local, original));
                    }
                }
            }
            _ => collect_ts_aliases(&child, source, out),
        }
    }
}

pub(crate) fn python_imported_names(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    // For `from X import a, b, c` or `from X import a as A, b as B`, walk
    // children skipping the module (dotted_name / module_name / relative_import)
    // and collect identifiers / aliased_import names.
    let mut out = Vec::new();
    let mut saw_import_kw = node.kind() == "import_statement"; // plain import: all names count
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import" => saw_import_kw = true,
            "from" => saw_import_kw = false, // `from` comes before `import`
            _ if !saw_import_kw => continue,
            "identifier" => {
                if let Ok(t) = child.utf8_text(source) {
                    out.push(t.to_string());
                }
            }
            "dotted_name" => {
                // Plain `import utils.format` — last segment is the bound name.
                if let Ok(t) = child.utf8_text(source) {
                    let last = t.split('.').next_back().unwrap_or(t);
                    out.push(last.to_string());
                }
            }
            "aliased_import" | "dotted_as_name" => {
                // `X as Y` — the bound name is the alias.
                if let Some(alias) = child.child_by_field_name("alias") {
                    if let Ok(t) = alias.utf8_text(source) {
                        out.push(t.to_string());
                    }
                } else if let Ok(t) = child.utf8_text(source) {
                    // Fallback: the name itself.
                    let after_as = t.rsplit(" as ").next().unwrap_or(t);
                    let last = after_as.rsplit('.').next().unwrap_or(after_as);
                    out.push(last.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn ts_js_imported_names(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    collect_ts_import_names(node, source, &mut out);
    out
}

/// v1.4 S-002 / AS-015..017 — collect names that carry the `type` modifier.
/// Three input shapes recognised:
/// - Whole-statement: an `import_statement` / `export_statement` whose
///   immediate children include the bare `type` keyword token. All names
///   inside it are type-only.
/// - Per-name: an `import_specifier` whose first non-whitespace token text
///   is `type` (tree-sitter-typescript represents this as an anonymous
///   `type` token before the name). Only that name is type-only.
/// - Re-export `export type { ... } from 'mod'`: same whole-statement
///   detection at the export_statement level.
pub(crate) fn ts_js_type_only_names(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let whole_statement_type_only = node_has_bare_type_keyword(node, source);
    collect_ts_type_only_names(node, source, whole_statement_type_only, &mut out);
    out
}

fn node_has_bare_type_keyword(node: &Node<'_>, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Anonymous `type` keyword token sits as a direct child of the
        // import_statement / export_statement when the form is whole-
        // statement type-only.
        if child.kind() == "type" {
            if let Ok(t) = child.utf8_text(source) {
                if t.trim() == "type" {
                    return true;
                }
            }
        }
    }
    false
}

fn collect_ts_type_only_names(
    node: &Node<'_>,
    source: &[u8],
    inherit_type_only: bool,
    out: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_specifier" | "export_specifier" => {
                let per_name_type_only =
                    inherit_type_only || node_has_bare_type_keyword(&child, source);
                if !per_name_type_only {
                    continue;
                }
                // Mirror collect_ts_import_names — prefer alias, fall back to name.
                let pushed = if let Some(alias) = child.child_by_field_name("alias") {
                    if let Ok(t) = alias.utf8_text(source) {
                        out.push(t.to_string());
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !pushed {
                    if let Some(name) = child.child_by_field_name("name") {
                        if let Ok(t) = name.utf8_text(source) {
                            out.push(t.to_string());
                        }
                    }
                }
            }
            _ => collect_ts_type_only_names(&child, source, inherit_type_only, out),
        }
    }
}

fn collect_ts_import_names(node: &Node<'_>, source: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_specifier" | "export_specifier" => {
                if let Some(alias) = child.child_by_field_name("alias") {
                    if let Ok(t) = alias.utf8_text(source) {
                        out.push(t.to_string());
                        continue;
                    }
                }
                if let Some(name) = child.child_by_field_name("name") {
                    if let Ok(t) = name.utf8_text(source) {
                        out.push(t.to_string());
                    }
                }
            }
            "identifier" | "property_identifier" => {
                if let Ok(t) = child.utf8_text(source) {
                    out.push(t.to_string());
                }
            }
            "namespace_import" => {
                let mut inner = child.walk();
                for gc in child.children(&mut inner) {
                    if gc.kind() == "identifier" {
                        if let Ok(t) = gc.utf8_text(source) {
                            out.push(t.to_string());
                        }
                    }
                }
            }
            _ => collect_ts_import_names(&child, source, out),
        }
    }
}

// --- Rust use-declaration helpers -----------------------------------------
//
// B1 — Rust IMPORTS_NAMED parity. tree-sitter-rust use_declaration shapes:
//   `use foo::Bar;`              → scoped_identifier(foo, Bar)
//   `use foo::Bar as B;`         → use_as_clause(scoped_identifier, B)
//   `use foo::{Bar, Baz};`       → scoped_use_list(foo, use_list[Bar, Baz])
//   `use foo::*;`                → use_wildcard(foo)
//   `use foo;`                   → identifier(foo)
//   `use crate::a::b::Foo;`      → scoped_identifier(scoped_identifier(...), Foo)
//
// `use_as_clause` and `scoped_identifier` use POSITIONAL children (no field
// names) — the alias / leaf is the LAST identifier child of the node.

pub(crate) fn rust_imported_names(node: &Node<'_>, source: &[u8]) -> Vec<String> {
    if node.kind() != "use_declaration" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for arg in node.named_children(&mut cursor) {
        rust_collect_use_names(&arg, source, &mut out);
    }
    out
}

pub(crate) fn rust_imported_aliases(node: &Node<'_>, source: &[u8]) -> Vec<(String, String)> {
    if node.kind() != "use_declaration" {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for arg in node.named_children(&mut cursor) {
        rust_collect_use_aliases(&arg, source, &mut out);
    }
    out
}

fn rust_collect_use_names(node: &Node<'_>, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "use_wildcard" => {
            // Glob — binds nothing nameable.
        }
        "use_as_clause" => {
            // Last identifier child = the alias = the bound local name.
            if let Some(name) = rust_last_identifier_text(node, source) {
                out.push(name);
            }
        }
        "scoped_identifier" => {
            // `foo::Bar::Baz` — leaf = bound name.
            if let Some(name) = rust_last_identifier_text(node, source) {
                out.push(name);
            }
        }
        "identifier" => {
            // Bare `use foo;`.
            if let Ok(t) = node.utf8_text(source) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
        "scoped_use_list" => {
            // `foo::{Bar, Baz, sub::*}` — recurse into the use_list children.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "use_list" {
                    let mut inner = child.walk();
                    for item in child.named_children(&mut inner) {
                        rust_collect_use_names(&item, source, out);
                    }
                }
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for item in node.named_children(&mut cursor) {
                rust_collect_use_names(&item, source, out);
            }
        }
        _ => {}
    }
}

fn rust_collect_use_aliases(node: &Node<'_>, source: &[u8], out: &mut Vec<(String, String)>) {
    match node.kind() {
        "use_as_clause" => {
            // Children: [path, alias_identifier]. Path's leaf = original;
            // alias = last identifier child of the use_as_clause itself.
            let mut path_leaf: Option<String> = None;
            let mut alias: Option<String> = None;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "scoped_identifier" => {
                        path_leaf = rust_last_identifier_text(&child, source);
                    }
                    "identifier" => {
                        // First-pass identifier child = path; later one = alias.
                        // tree-sitter-rust orders them: path-first OR a single
                        // identifier path. Use ORDER: assign first → path,
                        // second → alias.
                        if path_leaf.is_none() {
                            if let Ok(t) = child.utf8_text(source) {
                                path_leaf = Some(t.trim().to_string());
                            }
                        } else if let Ok(t) = child.utf8_text(source) {
                            alias = Some(t.trim().to_string());
                        }
                    }
                    _ => {}
                }
            }
            if let (Some(orig), Some(local)) = (path_leaf, alias) {
                if local != orig {
                    out.push((local, orig));
                }
            }
        }
        "scoped_use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "use_list" {
                    let mut inner = child.walk();
                    for item in child.named_children(&mut inner) {
                        rust_collect_use_aliases(&item, source, out);
                    }
                }
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for item in node.named_children(&mut cursor) {
                rust_collect_use_aliases(&item, source, out);
            }
        }
        _ => {}
    }
}

fn rust_last_identifier_text(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(t) = child.utf8_text(source) {
                last = Some(t.trim().to_string());
            }
        }
    }
    last
}

// --- TS/JS re-export helpers ----------------------------------------------

/// `export { X } from '…'` / `export * from '…'` → (path, names) when this
/// is a re-export node, None otherwise. Used by TS/JS LanguageSpec impls.
pub(crate) fn ts_js_extract_re_export(
    node: &Node<'_>,
    source: &[u8],
) -> Option<(String, Vec<String>)> {
    if node.kind() != "export_statement" {
        return None;
    }
    let path = extract_export_source(node, source)?;
    let mut names = Vec::new();
    collect_ts_import_names(node, source, &mut names);
    Some((path, names))
}

fn extract_export_source(node: &Node<'_>, source: &[u8]) -> Option<String> {
    // export_statement with a `source` field or a string child = re-export.
    if let Some(src_node) = node.child_by_field_name("source") {
        return literal_string_text(&src_node, source);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "string" | "string_literal") {
            return literal_string_text(&child, source);
        }
    }
    None
}

fn literal_string_text(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    let trimmed = text.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
