//! Java `LanguageSpec`. Grammar: `tree-sitter-java` 0.23 (pinned in
//! Cargo.toml + Cargo.lock per AS-016).
//!
//! v1.1-M4 S-001a (skeleton) — node-kind metadata + family classification +
//! `extract_import_path` / `extract_bases` extractors. Empty `callee_extractors`
//! / `ref_emitters` tables. Subsequent sub-units add behaviour:
//!   - S-001b: AS-001 CALLS happy path + AS-005 parse tolerance + java-tiny fixture
//!   - S-001c: AS-002 EXTENDS + AS-003 IMPORTS + AS-004 @Autowired REFERENCES
//!     and Lang-C6 imports.rs migration

use crate::references::{ParsedReference, RefKind};
use crate::{CalleeExtractor, LangFamily, LanguageSpec, RefEmitter};
use ga_core::Lang;
use tree_sitter::{Language, Node};

pub struct JavaLang;

// AS-016 checklist — AST node kinds tree-sitter-java 0.23 emits per category.
// Any grammar bump must update these or `grammar_drift.rs` turns red.
const SYMBOLS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "method_declaration",
    "constructor_declaration",
];
const IMPORTS: &[&str] = &["import_declaration"];
const CALLS: &[&str] = &["method_invocation", "object_creation_expression"];
const EXTENDS: &[&str] = &["class_declaration", "interface_declaration"];

// S-001b — Java `method_invocation` exposes the called method name on the
// `name` field (default `extract_standard_callee` reads `function`, which
// returns nil here, so the fallback `child(0)` would pick up the receiver
// `obj` — wrong). `object_creation_expression` exposes the constructed
// type on the `type` field. Both extractors mirror the JS/TS shared
// `new_expression` pattern.
const CALLEE_EXTRACTORS: &[(&str, CalleeExtractor)] = &[
    ("method_invocation", extract_method_invocation_callee),
    ("object_creation_expression", extract_object_creation_callee),
];

// S-001c (AS-004) — annotated field → REFERENCES edge to field type.
// Wired on `field_declaration` (not on the annotation node) so multiple
// annotations on the same field emit ONE ref, not N.
const REF_EMITTERS: &[(&str, RefEmitter)] = &[("field_declaration", extract_annotated_field_ref)];

impl LanguageSpec for JavaLang {
    fn lang(&self) -> Lang {
        Lang::Java
    }

    fn tree_sitter_lang(&self) -> Language {
        tree_sitter_java::LANGUAGE.into()
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

    /// Java `import com.example.foo.User;` → `["User"]` (trailing class).
    /// Wildcard `import com.example.util.*;` → `[]` (no specific name binds;
    /// caller resolves at call site against the package).
    /// Static `import static java.util.Collections.emptyList;`
    ///   → `["emptyList"]` (last segment of the FQN).
    fn extract_imported_names(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        // Wildcard imports contain an `asterisk` child after the FQN.
        let mut has_wildcard = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "asterisk" {
                has_wildcard = true;
                break;
            }
        }
        if has_wildcard {
            return Vec::new();
        }
        // Look up the (scoped_)identifier and take its last segment.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "scoped_identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if let Some(last) = text.rsplit('.').next() {
                            if !last.is_empty() {
                                return vec![last.to_string()];
                            }
                        }
                    }
                }
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if !text.is_empty() {
                            return vec![text.to_string()];
                        }
                    }
                }
                _ => {}
            }
        }
        Vec::new()
    }

    /// Java `import com.example.foo.Bar;` / `import com.example.util.*;` —
    /// the import_declaration node has a `scoped_identifier` (or `identifier`)
    /// child whose text is the FQN. Wildcard imports include an `asterisk`
    /// child. We return the FQN portion as written; caller decides whether to
    /// strip the `.*` suffix for package-level resolution.
    fn extract_import_path(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "scoped_identifier" | "identifier" => {
                    return child.utf8_text(source).ok().map(str::to_string);
                }
                _ => {}
            }
        }
        None
    }

    /// Java EXTENDS extraction:
    ///   - `class Admin extends User implements Printable, Cloneable`
    ///       → superclass field (single) + interfaces field (super_interfaces).
    ///   - `interface Printable extends Serializable, Cloneable`
    ///       → child node `extends_interfaces` (NOT exposed via a tree-sitter
    ///         field name in tree-sitter-java 0.23 — must walk children).
    /// Each base contributes one EXTENDS edge per AS-002.
    fn extract_bases(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        let mut bases = Vec::new();

        if let Some(sc) = node.child_by_field_name("superclass") {
            collect_type_identifiers(&sc, source, &mut bases);
        }
        if let Some(si) = node.child_by_field_name("interfaces") {
            collect_type_identifiers(&si, source, &mut bases);
        }
        // `interface X extends A, B` — `extends_interfaces` is a child node
        // without a field tag in tree-sitter-java 0.23.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "extends_interfaces" {
                collect_type_identifiers(&child, source, &mut bases);
            }
        }
        bases
    }

    /// Gap 4 — Java attributes: extract @Override → SymbolAttribute::Override
    /// (PR3 maps to is_override bool). Other annotations → Annotation(name)
    /// for DECORATES emission. Static keyword → Static attribute.
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
            if child.kind() != "modifiers" {
                continue;
            }
            let mut cc = child.walk();
            for m in child.children(&mut cc) {
                let mk = m.kind();
                if mk == "marker_annotation" || mk == "annotation" {
                    // @Override / @MyAnno("x") — extract the name identifier.
                    let mut anc = m.walk();
                    let name = m
                        .named_children(&mut anc)
                        .find(|c| matches!(c.kind(), "identifier" | "scoped_identifier"))
                        .and_then(|n| n.utf8_text(source).ok())
                        .map(|s| {
                            // Strip module prefix if scoped: javax.annotation.X → X
                            s.rsplit('.').next().unwrap_or(s).to_string()
                        });
                    if let Some(n) = name {
                        match n.as_str() {
                            "Override" => attrs.push(crate::SymbolAttribute::Override),
                            // Gap 5 — JUnit / TestNG markers
                            "Test" | "ParameterizedTest" | "RepeatedTest" => {
                                attrs.push(crate::SymbolAttribute::TestMarker)
                            }
                            _ => attrs.push(crate::SymbolAttribute::Annotation(n)),
                        }
                    }
                } else if mk == "static" {
                    attrs.push(crate::SymbolAttribute::Static);
                }
            }
            break;
        }
        attrs
    }

    /// PR5c2b — Java modifiers from `modifiers` block child. Skips
    /// annotations (those become DECORATES edges in PR8).
    fn extract_modifiers(&self, node: &Node<'_>, source: &[u8]) -> Vec<String> {
        if !matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return Vec::new();
        }
        crate::langs::shared::extract_modifiers_block(node, source)
    }

    /// PR5c2b — Java params: `formal_parameters` field. Each child is a
    /// `formal_parameter` with `type` + `name` fields (and optional
    /// `modifiers` for `final`).
    fn extract_params(&self, node: &Node<'_>, source: &[u8]) -> Vec<crate::ParsedParam> {
        if !matches!(
            node.kind(),
            "method_declaration" | "constructor_declaration"
        ) {
            return Vec::new();
        }
        crate::langs::shared::extract_params_by_container(node, source, "parameters")
    }

    /// PR5b — Java `method_declaration.type` is the return type. `void`
    /// surfaces explicitly (not the empty sentinel — it's an explicit type).
    /// Constructors omitted (`constructor_declaration` has no return type).
    fn extract_return_type(&self, node: &Node<'_>, source: &[u8]) -> Option<String> {
        if node.kind() != "method_declaration" {
            return None;
        }
        crate::langs::shared::extract_return_type_by_field(node, source, "type")
    }
}

/// `obj.findById(id)` / `Collections.emptyList()` / `helper()` →
/// callee_name = method name only. The `name` field on `method_invocation`
/// is always an `identifier` per tree-sitter-java grammar, so a single
/// child_by_field_name lookup is enough.
fn extract_method_invocation_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    name_node.utf8_text(source).ok().map(str::to_string)
}

/// `new User()` → "User"; `new com.example.Bar()` → "Bar"; generic types
/// `new List<String>()` → "List". Tree-sitter-java exposes the constructed
/// type on the `type` field, which can be a `type_identifier`, a
/// `scoped_type_identifier` (`pkg.Bar`), or a `generic_type` containing
/// either of the above.
fn extract_object_creation_callee(node: &Node<'_>, source: &[u8]) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    callee_from_type_node(&type_node, source)
}

fn callee_from_type_node(node: &Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" => node.utf8_text(source).ok().map(str::to_string),
        "scoped_type_identifier" | "scoped_identifier" => {
            let text = node.utf8_text(source).ok()?;
            text.rsplit('.').next().map(str::to_string)
        }
        "generic_type" => {
            // Inner is a (scoped_)type_identifier preceding `<...>`.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "type_identifier"
                        | "scoped_type_identifier"
                        | "identifier"
                        | "scoped_identifier"
                ) {
                    return callee_from_type_node(&child, source);
                }
            }
            None
        }
        _ => {
            let text = node.utf8_text(source).ok()?;
            text.rsplit('.').next().map(str::to_string)
        }
    }
}

/// AS-004 — emit one ParsedReference per annotated field. Wired on
/// `field_declaration` (not on `marker_annotation` / `annotation`) so a
/// field carrying multiple annotations (`@Autowired @Lazy private X x;`)
/// produces a single ref to `X`, not N.
///
/// Behavior:
///   - Walks `modifiers` child looking for `marker_annotation` /
///     `annotation`. If none → no emit.
///   - Reads the `type` field on field_declaration; resolves to a
///     callee-style trailing identifier via `callee_from_type_node`
///     (handles type_identifier / scoped_type_identifier / generic_type).
///   - Annotations on methods or classes never reach here — those are
///     `method_declaration` / `class_declaration` nodes, not
///     `field_declaration`.
fn extract_annotated_field_ref(
    node: &Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
) {
    if !field_has_annotation(node) {
        return;
    }
    let Some(type_node) = node.child_by_field_name("type") else {
        return;
    };
    let Some(target_name) = callee_from_type_node(&type_node, source) else {
        return;
    };
    out.push(ParsedReference {
        enclosing_symbol: enclosing.clone(),
        target_name,
        ref_site_line: (node.start_position().row as u32) + 1,
        ref_kind: RefKind::AnnotatedFieldType,
    });
}

fn field_has_annotation(field: &Node<'_>) -> bool {
    let mut cursor = field.walk();
    for child in field.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let mut inner = child.walk();
            for m in child.children(&mut inner) {
                if matches!(m.kind(), "marker_annotation" | "annotation") {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk a superclass / super_interfaces / extends_interfaces subtree and
/// collect trailing identifier names. `pkg.Base` → `Base` (strip module
/// prefix to match the Symbol-name layer the indexer resolves against).
fn collect_type_identifiers(node: &Node<'_>, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "type_identifier" | "identifier" => {
            if let Ok(text) = node.utf8_text(source) {
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
        }
        "scoped_type_identifier" | "scoped_identifier" => {
            if let Ok(text) = node.utf8_text(source) {
                let last = text.rsplit('.').next().unwrap_or(text);
                if !last.is_empty() {
                    out.push(last.to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_type_identifiers(&child, source, out);
            }
        }
    }
}
