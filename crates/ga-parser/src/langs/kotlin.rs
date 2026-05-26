//! Kotlin `LanguageSpec`. Grammar: `tree-sitter-kotlin-ng` 1.1 (pinned in
//! Cargo.toml + Cargo.lock per AS-016).
//!
//! v1.1-M4 sub-units:
//!   - S-002a: skeleton (node-kind metadata + family + empty extractors)
//!   - S-002b: AS-006 CALLS happy path (no per-lang CALLEE_EXTRACTORS —
//!     engine's default `extract_standard_callee` handles `call_expression`
//!     with identifier / navigation_expression children correctly via the
//!     rsplit('.') fallback) + AS-005-equiv parse tolerance + Lang-C2
//!     kotlin-tiny fixture.
//!   - S-002c: AS-007 extension fn + AS-008 suspend + AS-002-equiv EXTENDS
//!     + AS-003-equiv IMPORTS + AS-004-equiv @Inject REFERENCES
//!     (Lang-C7 AnnotatedFieldType).

use crate::references::{ParsedReference, RefKind};
use crate::{EnclosingScope, LangFamily, LanguageSpec, RefEmitter, SymbolAttribute};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct KotlinLang;

// AS-016 checklist — AST node kinds tree-sitter-kotlin-ng 1.1 emits per
// category. Probed against canonical fixtures (see grammar_drift.rs).
// Any grammar bump must update these or `grammar_drift.rs` turns red.
//
// Kotlin emits `class_declaration` for class / interface / enum / data /
// annotation class — distinguished by keyword children (`class` /
// `interface` / `class_modifier > enum|data|annotation`). `object` uses a
// separate top-level node `object_declaration`.
const SYMBOLS: &[&str] = &[
    "class_declaration",
    "object_declaration",
    "function_declaration",
];
// Note: tree-sitter-kotlin-ng uses `import` (not `import_declaration`).
const IMPORTS: &[&str] = &["import"];
const CALLS: &[&str] = &["call_expression"];
const EXTENDS: &[&str] = &["class_declaration", "object_declaration"];

// S-002c (Lang-C7 AS-004-equiv) — annotated property → REFERENCES edge to
// property type. Wired on `property_declaration` (not on the annotation
// node) so multiple annotations on the same property emit ONE ref, not N.
// Mirrors Java field_declaration emit-once semantics.
// Cross-lang sweep (2026-05-23) — mirror LANG-2 (PHP `Class::method()`).
// `Type.method()` companion-object calls emit a REFERENCES edge to `Type`
// so `ga_callers Type` surfaces invocation sites of any companion method.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[
    ("property_declaration", extract_annotated_property_ref),
    ("call_expression", extract_scoped_call_class_ref),
];

impl LanguageSpec for KotlinLang {
    fn lang(&self) -> Lang {
        Lang::Kotlin
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_kotlin_ng::LANGUAGE.into()
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

    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        REF_EMITTERS
    }

    /// AS-007 / AS-008 — Kotlin per-symbol attributes.
    ///
    /// AS-007 extension fns: `function_declaration` whose first
    /// `user_type` child precedes the `identifier` name field. Emit
    /// `SymbolAttribute::ExtendedReceiver(<raw type>)` (generic args
    /// stripped — `List<Int>.foo` → `ExtendedReceiver("List")`).
    ///
    /// AS-008 suspend fns: `function_declaration` whose `modifiers`
    /// child contains `function_modifier > suspend`. Emit
    /// `SymbolAttribute::Suspend`.
    ///
    /// Both attributes can co-occur on the same fn (Android pattern:
    /// `suspend fun Flow<T>.collectAll()`).
    fn extract_attributes(&self, node: &Node<'_>, source: &[u8]) -> Vec<SymbolAttribute> {
        if node.kind() != "function_declaration" {
            return Vec::new();
        }
        let mut attrs = Vec::new();
        // Existing (pre-Gap 4): Suspend + ExtendedReceiver.
        if has_suspend_modifier(node) {
            attrs.push(SymbolAttribute::Suspend);
        }
        if let Some(receiver) = extension_receiver_type(node, source) {
            attrs.push(SymbolAttribute::ExtendedReceiver(receiver));
        }
        // Gap 4 — `override` keyword + `@annotation`s in modifiers block.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "modifiers" {
                continue;
            }
            let mut cc = child.walk();
            for m in child.named_children(&mut cc) {
                let mk = m.kind();
                if mk.contains("annotation") {
                    let mut anc = m.walk();
                    let name = m
                        .named_children(&mut anc)
                        .find(|c| {
                            matches!(c.kind(), "identifier" | "user_type" | "simple_identifier")
                        })
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.rsplit('.').next().unwrap_or(s).to_string());
                    if let Some(n) = name {
                        attrs.push(SymbolAttribute::Annotation(n));
                    }
                    continue;
                }
                if let Ok(t) = m.utf8_text(source) {
                    match t.trim() {
                        "override" => attrs.push(SymbolAttribute::Override),
                        "static" => attrs.push(SymbolAttribute::Static),
                        _ => {}
                    }
                }
            }
            break;
        }
        attrs
    }

    /// AS-007 — extension fns enclose under `ExtendedType(<receiver>)`
    /// regardless of whether the fn is at top-level or nested in a class.
    /// Returns None for non-extension fns; walker falls back to the
    /// inherited Class scope.
    fn enclosing_for_symbol(&self, node: &Node<'_>, source: &[u8]) -> Option<EnclosingScope> {
        if node.kind() != "function_declaration" {
            return None;
        }
        extension_receiver_type(node, source).map(EnclosingScope::ExtendedType)
    }

    /// Kotlin EXTENDS extraction (AS-002-equiv).
    ///
    /// Tree-sitter shape:
    /// ```text
    /// class_declaration "class Admin : User(), Printable"
    ///   ├── delegation_specifiers
    ///   │     ├── delegation_specifier "User()"
    ///   │     │     └── constructor_invocation
    ///   │     │           └── user_type → identifier "User"
    ///   │     └── delegation_specifier "Printable"
    ///   │           └── user_type → identifier "Printable"
    /// ```
    /// Each `delegation_specifier` contributes one EXTENDS edge per
    /// AS-002-equiv. Qualified base `com.example.Base` → "Base" (last
    /// identifier in user_type's immediate children). Generic `List<T>`
    /// → "List" (raw, since type_arguments wraps the parameter scope).
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "delegation_specifiers" {
                continue;
            }
            let mut c2 = child.walk();
            for spec in child.children(&mut c2) {
                if spec.kind() != "delegation_specifier" {
                    continue;
                }
                if let Some(name) = base_from_delegation_specifier(&spec, source) {
                    bases.push(name);
                }
            }
        }
        bases
    }

    /// Kotlin IMPORTS extraction (AS-003-equiv).
    ///
    /// `import org.foo.Bar`        → target_path = "org.foo.Bar"
    /// `import org.foo.*`          → target_path = "org.foo" (qualified_identifier
    ///                                portion only — `.*` are sibling tokens)
    /// `import org.foo.Bar as B`   → target_path = "org.foo.Bar" (alias child
    ///                                is a sibling identifier)
    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "qualified_identifier" {
                return child.utf8_text(source).ok().map(str::to_string);
            }
        }
        None
    }

    /// Kotlin imported-name extraction (AS-003-equiv binding rule).
    ///
    /// - Wildcard `import pkg.*` → `[]` (no specific name binds at the
    ///   call site; resolution is package-level).
    /// - Aliased `import pkg.X as Y` → `["Y"]` (LOCAL name — what shows up
    ///   at call sites in the importing file).
    /// - Plain `import pkg.X` → `["X"]` (last segment of FQN).
    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut has_wildcard = false;
        let mut alias: Option<String> = None;
        let mut after_as = false;
        let mut qid_text: Option<String> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "*" => has_wildcard = true,
                "as" => after_as = true,
                "qualified_identifier" => {
                    qid_text = child.utf8_text(source).ok().map(str::to_string);
                }
                "identifier" if after_as => {
                    if let Ok(text) = child.utf8_text(source) {
                        if !text.is_empty() {
                            alias = Some(text.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        if has_wildcard {
            return Vec::new();
        }
        if let Some(a) = alias {
            return vec![a];
        }
        if let Some(t) = qid_text {
            if let Some(last) = t.rsplit('.').next() {
                if !last.is_empty() {
                    return vec![last.to_string()];
                }
            }
        }
        Vec::new()
    }

    /// Gap 3 — Kotlin return type. tree-sitter-kotlin-ng `function_declaration`
    /// positional shape: `fun` → `identifier` (name) → `function_value_parameters`
    /// → [optional `:` + return-type node] → `function_body`. Return type is
    /// the named child between function_value_parameters and function_body.
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "function_declaration" {
            return None;
        }
        let mut cursor = node.walk();
        let named: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        let params_idx = named
            .iter()
            .position(|n| n.kind() == "function_value_parameters")?;
        if params_idx + 1 >= named.len() {
            return None;
        }
        let candidate = named[params_idx + 1];
        // function_body is the body (no return type); skip it.
        if candidate.kind() == "function_body" {
            return None;
        }
        let text = candidate.utf8_text(source).ok()?.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    /// PR5c2b — Kotlin modifiers from `modifiers` block child. Includes
    /// `function_modifier` (suspend / inline / tailrec / external),
    /// `visibility_modifier` (public / private / internal), `inheritance_modifier`
    /// (open / final / abstract / override).
    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if node.kind() != "function_declaration" {
            return Vec::new();
        }
        let mut mods = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "modifiers" {
                continue;
            }
            let mut cc = child.walk();
            for m in child.named_children(&mut cc) {
                let mk = m.kind();
                // Annotations → DECORATES (PR8 scope), skip.
                if mk.contains("annotation") {
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

    /// PR5c2b — Kotlin params: `function_value_parameters` field. Each
    /// `parameter` child has nested name + type structure.
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if node.kind() != "function_declaration" {
            return Vec::new();
        }
        // Try common Kotlin field names; fall back to walking children.
        for field in &["parameters", "function_value_parameters"] {
            if node.child_by_field_name(field).is_some() {
                return crate::langs::shared::extract_params_by_container(node, source, field);
            }
        }
        // Fallback: find function_value_parameters by walking.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "function_value_parameters" {
                let mut out = Vec::new();
                let mut cc = child.walk();
                for p in child.named_children(&mut cc) {
                    if !p.kind().contains("parameter") {
                        continue;
                    }
                    // tree-sitter-kotlin-ng: parameter has positional
                    // children — `identifier` (name) + `:` + `user_type`
                    // (type). No field names.
                    let mut pc = p.walk();
                    let mut child_iter = p.named_children(&mut pc);
                    let name = child_iter
                        .next()
                        .filter(|c| c.kind() == "identifier" || c.kind() == "simple_identifier")
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let ty = child_iter
                        .next()
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| {
                            let t = s.trim();
                            t.strip_prefix(':').unwrap_or(t).trim().to_string()
                        })
                        .unwrap_or_default();
                    if !name.is_empty() {
                        out.push(crate::ParsedParam {
                            name,
                            type_: ty,
                            default_value: String::new(),
                        });
                    }
                }
                return out;
            }
        }
        Vec::new()
    }
}

/// Walk a `delegation_specifier` (either `constructor_invocation` form
/// `Foo()` or bare `user_type` form `Bar`) and return the trailing
/// identifier of its `user_type`. Module prefix stripped per AS-002 spec.
fn base_from_delegation_specifier(spec: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = spec.walk();
    for child in spec.children(&mut cursor) {
        let user_type = match child.kind() {
            "user_type" => Some(child),
            "constructor_invocation" => {
                let mut cc = child.walk();
                let found = child.children(&mut cc).find(|c| c.kind() == "user_type");
                found
            }
            _ => None,
        };
        if let Some(ut) = user_type {
            return last_immediate_identifier(&ut, source);
        }
    }
    None
}

/// Return the LAST immediate `identifier` child of a `user_type` node.
/// For `com.example.Base` → "Base"; for raw `User` → "User"; for
/// `Container<T>` → "Container" (type_arguments contains nested user_type
/// for `T` but we ignore non-immediate-identifier children).
fn last_immediate_identifier(user_type: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = user_type.walk();
    for child in user_type.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(text) = child.utf8_text(source) {
                if !text.is_empty() {
                    last = Some(text.to_string());
                }
            }
        }
    }
    last
}

/// AS-004-equiv (Lang-C7) — emit one ParsedReference per annotated
/// property. Wired on `property_declaration` (not on the annotation
/// node itself) so a property carrying multiple annotations
/// (`@Inject @Lazy lateinit var x: T`) produces a SINGLE ref to `T`,
/// not N.
///
/// Behavior:
///   - Walks `modifiers` child for `annotation` nodes. If none → no emit.
///   - Reads the `variable_declaration` child's `user_type`; resolves to
///     a callee-style trailing identifier via `last_immediate_identifier`
///     (handles type_identifier / scoped FQN / generic raw type).
///   - Annotations on functions or classes never reach here — those are
///     `function_declaration` / `class_declaration` nodes, not
///     `property_declaration`.
fn extract_annotated_property_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    if !property_has_annotation(node) {
        return;
    }
    let Some(user_type) = property_type_node(node) else {
        return;
    };
    let Some(target_name) = last_immediate_identifier(&user_type, source) else {
        return;
    };
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name,
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::AnnotatedFieldType,
    });
}

fn property_has_annotation(node: &Node<'_>) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner = child.walk();
            for m in child.children(&mut inner) {
                if m.kind() == "annotation" {
                    return true;
                }
            }
        }
    }
    false
}

/// AS-008 — walk `function_declaration > modifiers > function_modifier`
/// children for a `suspend` keyword token.
fn has_suspend_modifier(node: &Node<'_>) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "modifiers" {
            continue;
        }
        let mut inner = child.walk();
        for m in child.children(&mut inner) {
            if m.kind() != "function_modifier" {
                continue;
            }
            let mut tcursor = m.walk();
            for t in m.children(&mut tcursor) {
                if t.kind() == "suspend" {
                    return true;
                }
            }
        }
    }
    false
}

/// AS-007 — return the receiver type's raw name when this
/// `function_declaration` is an extension fn, else None.
///
/// Tree-sitter shape:
/// ```text
/// function_declaration "fun String.isEmail(): Boolean"
///   ├── (optional modifiers)
///   ├── fun
///   ├── user_type "String"           ← the receiver type
///   │     └── identifier "String"
///   ├── .
///   ├── identifier "isEmail"         ← the function name
///   └── ...
/// ```
/// Detection: a `user_type` child appearing BEFORE the name `identifier`.
/// (Plain fns have no `user_type` before the name.)
fn extension_receiver_type(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let mut user_type: Option<Node> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "user_type" => {
                // First user_type before the identifier marks the receiver.
                user_type = Some(child);
            }
            "identifier" => {
                // Reached the function name — if we already saw a user_type
                // before it, that user_type is the receiver.
                return user_type.and_then(|ut| last_immediate_identifier(&ut, source));
            }
            _ => {}
        }
    }
    None
}

fn property_type_node<'t>(node: &Node<'t>) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declaration" {
            let mut inner = child.walk();
            for c in child.children(&mut inner) {
                if c.kind() == "user_type" {
                    return Some(c);
                }
            }
        }
    }
    None
}

/// Cross-lang sweep (2026-05-23) — mirror LANG-2 PHP `Class::method()`.
/// Emit a REFERENCES edge to the receiver of a Kotlin `Type.method(args)`
/// companion-object call.
///
/// Tree-sitter-kotlin-ng `call_expression` shape for `MyClass.staticFn(42)`:
///   call_expression
///     navigation_expression
///       identifier "MyClass"
///       .
///       identifier "staticFn"
///     value_arguments(...)
///
/// For nested paths (`pkg.Util.fn()`) the navigation_expression nests; we
/// take the LAST identifier of the inner receiver portion (i.e. the identifier
/// immediately before the trailing method name).
///
/// Skips `this` / `super` / `it` keywords and stdlib types listed in
/// `is_kotlin_stdlib_type`.
fn extract_scoped_call_class_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    let mut cursor = node.walk();
    let first_child = node.children(&mut cursor).next();
    let Some(nav) = first_child else { return };
    if nav.kind() != "navigation_expression" {
        return;
    }
    // Collect immediate identifier children of the outer navigation_expression
    // and the nested navigation_expression (one level deep) so qualified paths
    // like `pkg.Util.fn()` still surface `Util` as receiver.
    let receiver = navigation_receiver_identifier(&nav, source);
    let Some(receiver_text) = receiver else {
        return;
    };
    if matches!(receiver_text.as_str(), "this" | "super" | "it") {
        return;
    }
    if !receiver_text
        .chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
    {
        return;
    }
    if is_kotlin_stdlib_type(&receiver_text) {
        return;
    }
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name: receiver_text,
        ref_site_line: (nav.start_position().row as u32) + 1,
        ref_kind: RefKind::TypePosition,
    });
}

/// Given a navigation_expression node, return the receiver identifier (the
/// segment immediately before the trailing method-name identifier). Walks one
/// level into a nested navigation_expression to support qualified paths.
fn navigation_receiver_identifier(nav: &Node<'_>, source: &[u8]) -> Option<String> {
    // Collect immediate identifier children.
    let mut idents: Vec<Node> = Vec::new();
    let mut nested: Option<Node> = None;
    let mut cursor = nav.walk();
    for child in nav.children(&mut cursor) {
        match child.kind() {
            "identifier" => idents.push(child),
            "navigation_expression" => nested = Some(child),
            _ => {}
        }
    }
    // Simple case `MyClass.fn`: two identifiers → first is receiver.
    if idents.len() >= 2 {
        return idents[idents.len() - 2]
            .utf8_text(source)
            .ok()
            .map(str::to_string);
    }
    // Nested case `pkg.Util.fn`: the inner nav holds the qualified receiver;
    // the outer nav contributes only the trailing method identifier. Take the
    // last immediate identifier of the inner nav as the receiver.
    if let Some(inner) = nested {
        let mut inner_cursor = inner.walk();
        let mut last: Option<String> = None;
        for c in inner.children(&mut inner_cursor) {
            if c.kind() == "identifier" {
                if let Ok(t) = c.utf8_text(source) {
                    last = Some(t.to_string());
                }
            }
        }
        return last;
    }
    None
}

/// Kotlin stdlib types whose static-call receivers would dominate the graph.
fn is_kotlin_stdlib_type(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Int"
            | "Long"
            | "Double"
            | "Float"
            | "Boolean"
            | "Char"
            | "Byte"
            | "Short"
            | "Any"
            | "Unit"
            | "Nothing"
            | "List"
            | "Map"
            | "Set"
            | "MutableList"
            | "MutableMap"
            | "MutableSet"
            | "Array"
            | "Math"
            | "System"
            | "Throwable"
            | "Exception"
            | "RuntimeException"
    )
}
