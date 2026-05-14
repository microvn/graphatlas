#![allow(clippy::doc_lazy_continuation, clippy::doc_overindented_list_items)]

//! Parser framework — Foundation S-004.
//!
//! The [`LanguageSpec`] trait owns per-language node-kind predicates +
//! extraction helpers. Shared traversal logic lives in [`walker`]. Per-lang
//! impls live in `langs::{py,ts,js,go,rs}`.
//!
//! Design ported from `rust-poc/src/main.rs:234-400` (predicates) plus
//! `:956` (tree walker). Trait-based shape replaces the big per-lang match
//! statements (PLAN R23 DRY goal: ~800 LoC vs ~2500 LoC copy-paste).

pub mod calls;
pub mod change_detect;
pub mod extends;
pub mod imports;
pub mod langs;
pub mod limits;
pub mod logs;
pub mod merkle;
pub mod parallel_reparse;
pub mod references;
pub mod staleness;
pub mod threshold;
pub mod walk;
pub mod walker;

pub use calls::{extract_calls, extract_calls_from_tree, ParsedCall};
pub use extends::{extract_extends, extract_extends_from_tree, ParsedExtends};
pub use imports::{extract_imports, extract_imports_from_tree, ParsedImport};
pub use limits::{parse_file_bytes, parse_file_tree, LimitConfig, ParseOutcome, ParseTreeOutcome};
pub use references::{extract_references, extract_references_from_tree, ParsedReference, RefKind};

// v1.1-M4 (S-005a D2) — fn-pointer dispatch surface for the LanguageSpec
// trait extension. See `lang_spec_extension.rs` test for design rationale.

/// Handler signature for callee-name extraction. Per-lang `LanguageSpec`
/// registers one of these per node kind via `callee_extractors()`. The
/// engine (`calls.rs`) looks up the table by node kind and dispatches —
/// no `match Lang::*` branching inside the engine.
pub type CalleeExtractor = fn(node: &tree_sitter::Node<'_>, source: &[u8]) -> Option<String>;

/// Handler signature for value-reference emission. Per-lang `LanguageSpec`
/// registers one of these per structural node kind via `ref_emitters()`.
/// Mirrors the existing per-lang helpers in `references.rs` (e.g.
/// `emit_go_keyed_element`, `emit_rust_call_arg_identifiers`) which D3
/// migrates out of the engine into per-lang impls.
pub type RefEmitter = fn(
    node: &tree_sitter::Node<'_>,
    source: &[u8],
    enclosing: &Option<String>,
    out: &mut Vec<ParsedReference>,
);

/// Cross-cutting language family — groups langs that share dispatch
/// confidence + annotation handling + idiomatic patterns. Used by indexer
/// and bench layers for policies like "DynamicScripting → reduce confidence".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LangFamily {
    /// JVM + .NET typed-managed langs (Java, Kotlin, Scala, C#, F#).
    /// Annotations as references; type-safe dispatch confidence.
    StaticManaged,
    /// JavaScript-family (JS, TS, JSX, TSX) — closures, prototype, JSX.
    JsLike,
    /// Python, Ruby, PHP, Lua — duck-typed, metaprogramming, lower confidence
    /// on indirect dispatch.
    DynamicScripting,
    /// Rust — borrow-checked systems, macros, traits.
    SystemsRust,
    /// Go — duck-typed interfaces, no inheritance.
    SystemsGo,
    /// Reserved for v2+ C / C++ / Zig — preprocessor + headers.
    SystemsCfamily,
    /// Default for unclassified langs. Lang impls override on construction.
    Other,
}

/// Convention-pair pattern for impact analysis. Per-lang declares the
/// idiomatic source/test mirror (Rails: `app/models/X.rb` ↔ `spec/models/X_spec.rb`).
/// `{name}` is the substitution token. Indexer composes via per-lang patterns.
#[derive(Debug, Clone, Copy)]
pub struct ConventionPair {
    pub src_pattern: &'static str,
    pub test_pattern: &'static str,
    pub confidence: f32,
}

/// Per-symbol attribute bag — open extension point so adding Kotlin
/// `suspend`, C# `partial`, Java `@Service` doesn't churn `SymbolKind`.
/// `ParsedSymbol.attributes` consumes this in D5.
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolAttribute {
    Async,
    Suspend,
    Partial,
    Static,
    Override,
    Const,
    /// Java/Kotlin/C# annotation (e.g., "Service", "Autowired", "Composable").
    /// String, not &'static str — annotations come from source text.
    Annotation(String),
    /// Python/JS/Ruby decorator (e.g., "app.route", "observable", "define_method").
    /// Gap 6 / AS-016 — `args` is the raw source text of the decorator's
    /// argument list, paren-stripped (e.g. `'/users', methods=['GET']` for
    /// `@app.route('/users', methods=['GET'])`). Empty when the decorator
    /// has no parens (`@my_decorator`). Indexer surfaces via DECORATES
    /// edge `decorator_args` column. Tools-C14 sanitization applies.
    Decorator {
        name: String,
        args: String,
    },
    /// Kotlin / Rust extension function — receiver type the function extends.
    /// E.g., `fun String.foo()` → `ExtendedReceiver("String")`.
    ExtendedReceiver(String),
    /// Gap 5 / AS-012 — parser-time test marker. Rust `#[test]`, Java
    /// `@Test`, Python/Ruby `def test_*`, Go `func TestX`. Indexer maps
    /// to `Symbol.is_test_marker = true` (Tools-C3 complements runtime
    /// `is_test_path` filename heuristic).
    TestMarker,
}

use ga_core::{Lang, Result, Symbol, SymbolKind};
use tree_sitter::{Language, Node, Parser};

/// Typed enclosing scope for a `ParsedSymbol`. S-005a D5 — replaces the
/// pre-D5 `enclosing_class: Option<String>` (ambiguous between class /
/// module / namespace / extension receiver). Phase C lang stories use
/// the typed variant directly: Kotlin `fun String.foo()` →
/// `ExtendedType("String")`, C# `namespace App` → `Namespace("App")`,
/// Python module-level → `Module(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnclosingScope {
    /// OOP class, struct, trait, interface, enum (treated alike for
    /// "method belongs to class X" semantics).
    Class(String),
    /// Kotlin / Rust extension function receiver. `fun String.isEmail()`
    /// → `ExtendedType("String")`.
    ExtendedType(String),
    /// Python / Ruby / JS module scope.
    Module(String),
    /// C# / C++ namespace.
    Namespace(String),
}

impl EnclosingScope {
    /// Inner name (without the variant tag). Used by `into_symbol`
    /// to map back to `ga_core::Symbol.enclosing: Option<String>` —
    /// downstream consumers like indexer KG-9 treat enclosing as an
    /// opaque name string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Class(name)
            | Self::ExtendedType(name)
            | Self::Module(name)
            | Self::Namespace(name) => name,
        }
    }
}

/// Symbol extracted from source, pre-resolution.
/// Equivalent to `rust-poc::ParsedSymbol` (main.rs:107).
///
/// S-005a D5 shape additions:
/// - `enclosing: Option<EnclosingScope>` (typed; was `enclosing_class: Option<String>`)
/// - `attributes: Vec<SymbolAttribute>` for modifiers (suspend/partial/async/annotations)
/// - `confidence: f32` for polymorphic / metaprogramming langs (Ruby
///   `define_method` → 0.6 per Tools-C11)
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 1-based line number of the definition's first line.
    pub line: u32,
    /// 1-based line number of the definition's last line. v1.1 schema v3
    /// addition powering `ga_large_functions` — line span = `line_end -
    /// line + 1`. Falls back to `line` for synthetic / metaprogramming
    /// symbols where no source range is meaningful.
    pub line_end: u32,
    pub enclosing: Option<EnclosingScope>,
    pub attributes: Vec<SymbolAttribute>,
    pub confidence: f32,
    /// PR5a / Tools-C2 — function/method parameter count. `None` for
    /// non-function symbols (class / struct / trait / module); indexer
    /// maps `None` → `-1` DDL DEFAULT (unknown sentinel). Walker populates
    /// from `LanguageSpec::extract_arity` per emitted symbol.
    pub arity: Option<i64>,
    /// PR5b / Tools-C2 — function return type as raw source text (no
    /// leading `->` or `:`). `None` for non-functions, dynamic langs, or
    /// unannotated funcs; indexer maps `None` → empty `''` DDL DEFAULT
    /// (unknown sentinel). Walker populates from
    /// `LanguageSpec::extract_return_type`.
    pub return_type: Option<String>,
    /// PR5c1 — modifiers (Rust pub/async, Java public/static, etc.).
    /// Empty in PR5c1 (per-lang extractors deferred to PR5c2). Stored
    /// as STRING[] in lbug.
    pub modifiers: Vec<String>,
    /// PR5c1 — function parameter list as STRUCT[]. Empty in PR5c1
    /// (per-lang extractors deferred to PR5c2).
    pub params: Vec<ParsedParam>,
}

/// PR5c1 — parameter struct entry. Mirrors lbug DDL
/// `STRUCT(name STRING, type STRING, default_value STRING)`. Per
/// Tools-C2 — empty `type` / `default_value` = unknown sentinel
/// (Python without type annotations, dynamic langs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedParam {
    pub name: String,
    pub type_: String,
    pub default_value: String,
}

impl ParsedSymbol {
    pub fn into_symbol(self, file: impl Into<String>) -> Symbol {
        let file = file.into();
        let id = format!("{file}:{}:{}", self.line, self.name);
        Symbol {
            id,
            name: self.name,
            kind: self.kind,
            file,
            line: self.line,
            line_end: self.line_end,
            enclosing: self.enclosing.map(|e| e.as_str().to_string()),
        }
    }
}

/// Per-language extraction contract. One impl per supported `Lang`.
/// Ports the match arms from `rust-poc/src/main.rs` {234..400} into
/// methods on a trait so we can test + extend per lang cleanly.
pub trait LanguageSpec: Send + Sync {
    fn lang(&self) -> Lang;

    /// tree-sitter `Language` for this spec. Parser::set_language consumer.
    fn tree_sitter_lang(&self) -> Language;

    // AS-010 canonical node-kind checklists ----------------------------------
    //
    // These return the full list of AST node kinds this language recognizes
    // for each category. The `is_*_node` predicates below default-derive from
    // these lists so the two APIs cannot drift.

    fn symbol_node_kinds(&self) -> &'static [&'static str];
    fn import_node_kinds(&self) -> &'static [&'static str];
    fn call_node_kinds(&self) -> &'static [&'static str];
    fn extends_node_kinds(&self) -> &'static [&'static str];

    // Predicates (default impls, rarely overridden) --------------------------

    fn is_symbol_node(&self, kind: &str) -> bool {
        self.symbol_node_kinds().contains(&kind)
    }
    fn is_import_node(&self, kind: &str) -> bool {
        self.import_node_kinds().contains(&kind)
    }
    fn is_call_node(&self, kind: &str) -> bool {
        self.call_node_kinds().contains(&kind)
    }
    fn is_extends_node(&self, kind: &str) -> bool {
        self.extends_node_kinds().contains(&kind)
    }

    // Per-lang extraction helpers (AS-010) -----------------------------------
    //
    // Default impls return empty so langs that don't need them (e.g. Go has no
    // class-style inheritance) stay noise-free.

    /// Given a node matching `is_extends_node`, return the names of the types
    /// it extends / implements. Ported from rust-poc/src/main.rs:286-339.
    fn extract_bases(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<String> {
        Vec::new()
    }

    /// Given a node matching `is_import_node`, return the target path / module.
    /// Ported from rust-poc/src/main.rs:382-417.
    fn extract_import_path(&self, _node: &Node<'_>, _source: &[u8]) -> Option<String> {
        None
    }

    // -----------------------------------------------------------------------
    // v1.1-M4 (S-005a D2) — fn-pointer dispatch slots + cross-cutting hooks.
    // Defaults are empty / Other so existing 5 langs auto-inherit. Engine
    // (calls.rs / references.rs) reads these tables instead of `match Lang::*`
    // branching once D3/D4 migration lands.
    // -----------------------------------------------------------------------

    /// Per-node-kind callee-name extractor table. Default empty — engine
    /// falls back to standard call-expression handling.
    fn callee_extractors(&self) -> &'static [(&'static str, CalleeExtractor)] {
        &[]
    }

    /// Per-node-kind value-reference emitter table. Default empty — engine
    /// applies generic structural emission (pair / array / shorthand).
    fn ref_emitters(&self) -> &'static [(&'static str, RefEmitter)] {
        &[]
    }

    /// Cross-cutting language family classification. Default `Other`;
    /// individual lang impls override.
    fn family(&self) -> LangFamily {
        LangFamily::Other
    }

    /// Idiomatic source/test mirror conventions for impact analysis.
    /// Default empty; langs with strong conventions (Rails, Django, NestJS)
    /// override.
    fn convention_pairs(&self) -> &'static [ConventionPair] {
        &[]
    }

    /// Per-symbol attributes (suspend, partial, async, annotations, ...).
    /// Default empty — most langs have none. Java/Kotlin/C# override when
    /// their per-lang impls land. Called by `walker.rs::walk_node` for
    /// every symbol-emitting node.
    fn extract_attributes(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<SymbolAttribute> {
        Vec::new()
    }

    /// v1.3 PR4 — per-language `qualified_name` chain formatter (S-002 Tools-C1).
    ///
    /// Returns the chain part of `Symbol.qualified_name` — the indexer
    /// prepends `{rel_path}::` so this method only owns enclosing-chain +
    /// separator semantics.
    ///
    /// Default: `.` separator (Python / TS / JS / Go / Java / Kotlin / C#).
    /// Overrides:
    /// - Rust: `::` separator
    /// - Ruby: `#` separator
    ///
    /// Java/Kotlin/C# `(arity)` overload suffix deferred to PR5 (needs
    /// signature extraction). Until then, overloaded methods collide and
    /// the indexer's AS-006 dedup appends `#dup<N>` — still UNIQUE per
    /// AT-001 audit.
    fn format_qualified_name(&self, name: &str, enclosing: Option<&EnclosingScope>) -> String {
        match enclosing {
            Some(scope) => format!("{}.{}", scope.as_str(), name),
            None => name.to_string(),
        }
    }

    /// PR5c2 — extract per-symbol modifiers (Rust pub/async/unsafe, Java
    /// public/static, Kotlin suspend, etc.). Returns raw keyword text in
    /// source order. Default: empty. Per-lang overrides where applicable.
    fn extract_modifiers(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<String> {
        Vec::new()
    }

    /// PR5c2 — extract function/method parameter list as `ParsedParam`
    /// entries. Each carries `name` + `type_` (raw text, empty sentinel
    /// per Tools-C2 for unannotated dynamic langs) + `default_value` (empty
    /// when absent). Default: empty. Per-lang overrides per AST shape.
    fn extract_params(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<ParsedParam> {
        Vec::new()
    }

    /// PR5b — extract return type as raw source text from a function/method node.
    ///
    /// Default returns `None`. Per-lang impls override based on the
    /// tree-sitter grammar's field name (`return_type` / `result` / `type`).
    /// Returns `Some(text)` for explicitly typed returns (incl. `void` for
    /// Java), `None` for unannotated / dynamic langs (Tools-C2 maps to
    /// empty `''` sentinel at the indexer layer). Returned text MUST NOT
    /// include leading `->` or `:` arrows — return raw type only.
    fn extract_return_type(&self, _node: &Node<'_>, _source: &[u8]) -> Option<String> {
        None
    }

    /// PR5a — extract arity (parameter count) from a function/method node.
    ///
    /// Default heuristic: find the first child whose `kind()` contains
    /// "parameter" (covers `parameters`, `formal_parameters`, `parameter_list`,
    /// `method_parameters`, `function_value_parameters` across tree-sitter
    /// grammars for the 9 wired langs) and count its named children. Returns
    /// `None` for non-function symbols (class / struct / trait / module) so
    /// the indexer keeps the DDL DEFAULT -1 sentinel per Tools-C2.
    ///
    /// Tools-C2 — `arity == -1` means "unknown / not a function". `0` is the
    /// universal-truth nullary arity. Downstream filters (rename_safety,
    /// large_functions) distinguish via `WHERE s.arity >= 0`.
    fn extract_arity(&self, node: &Node<'_>, _source: &[u8]) -> Option<i64> {
        // Only function-like symbols carry arity. Heuristic: kind contains
        // "function" or "method" or "fn" — broad enough for tree-sitter's
        // per-lang naming (rust function_item, py function_definition,
        // ts function_declaration / method_definition, go function_declaration
        // / method_declaration, java method_declaration, kotlin
        // function_declaration, csharp method_declaration, ruby method).
        let kind = node.kind();
        let is_fn_like = kind.contains("function")
            || kind.contains("method")
            || kind == "function_item"
            || kind == "constructor_declaration";
        if !is_fn_like {
            return None;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            // Match `parameters`, `formal_parameters`, `parameter_list`,
            // `method_parameters`, `function_value_parameters`. Exclude
            // `type_parameters` (generics) and `parameter` (singular —
            // not the list).
            if (ck.contains("parameters") || ck == "parameter_list")
                && !ck.contains("type_parameters")
            {
                let mut cc = child.walk();
                let count = child.named_children(&mut cc).count() as i64;
                return Some(count);
            }
        }
        // Function-like node with no parameter list found → assume nullary.
        // Covers Ruby `def foo` (no parens, no parameter node when zero
        // params) and Kotlin parameterless lambdas.
        Some(0)
    }

    /// PR4 — per-lang container-name fallback for nodes without a `name` field.
    ///
    /// Rust `impl_item` has a `type` field (e.g., `impl Foo { ... }` →
    /// `type = "Foo"`) but no `name` field, so `name_from_node` returns
    /// None and the walker doesn't track Foo as enclosing. Result: AS-005
    /// class (a) — `fn fmt` inside `impl Display for Foo` and inside
    /// `impl Display for Bar` would share the same qualified_name (no
    /// receiver disambiguation), forcing every fn-pair into AS-006 dedup.
    ///
    /// Lang impls override this to extract the container's type/receiver
    /// name from the appropriate field. Default returns None — most langs
    /// (Python `class Foo:`, Java `class Foo`) use the `name` field
    /// already covered by `name_from_node`.
    fn container_name_fallback(&self, _node: &Node<'_>, _source: &[u8]) -> Option<String> {
        None
    }

    /// Per-symbol enclosing-scope override. Called by `walker.rs::walk_node`
    /// for every symbol-emitting node — when this returns `Some(scope)`,
    /// the walker uses it for THIS symbol's `enclosing` field instead of
    /// the inherited Class scope. Children still see the inherited enclosing
    /// (no propagation) — this hook only affects the current node.
    ///
    /// Use case: Kotlin extension functions `fun String.foo()` need
    /// `EnclosingScope::ExtendedType("String")` regardless of whether the
    /// fn is at top-level or nested in a class. Default `None` — most
    /// langs let walker's Class-tracking handle enclosing.
    fn enclosing_for_symbol(&self, _node: &Node<'_>, _source: &[u8]) -> Option<EnclosingScope> {
        None
    }

    /// v1.1-M4 S-004c (AS-014) — synthesize a symbol from a non-symbol node.
    /// Use case: Ruby `define_method(:foo) { ... }` defines a method named
    /// `foo` from a `call` node (NOT a symbol_node). Confidence is reduced
    /// per Tools-C11 because the symbol's existence depends on runtime
    /// dispatch (method-name supplied as an argument to a call).
    ///
    /// Walker invokes this on EVERY node during traversal — per-lang impl
    /// returns None for nodes it doesn't recognize (which is most of them).
    /// Default None — langs without metaprogramming inherit no behavior.
    fn extract_synthetic_symbol(&self, _node: &Node<'_>, _source: &[u8]) -> Option<ParsedSymbol> {
        None
    }

    /// v1.1-M4 S-001c (Lang-C6 migration) — given an import-node, return the
    /// list of names this import binds at the call site. Default empty;
    /// per-lang impls override:
    ///   - Python `from X import a, b as B` → `["a", "B"]`
    ///   - TS/JS  `import { x, y as Y } from 'm'` → `["x", "Y"]`
    ///   - Java   `import com.example.User;` → `["User"]`; wildcard → `[]`
    fn extract_imported_names(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<String> {
        Vec::new()
    }

    /// v1.1-M4 S-001c (Lang-C6 migration) — given an import-node, return
    /// `(local, original)` alias pairs. Default empty; only Python and
    /// TS/JS impls override (Foundation-C16). Java has no `as` alias form
    /// — wildcard imports are package-bound, never aliased.
    fn extract_imported_aliases(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<(String, String)> {
        Vec::new()
    }

    /// v1.1-M4 S-001c (Lang-C6 migration) — detect a re-export node
    /// (`export { X } from '...'` / `export * from '...'`) and return
    /// `(target_path, exported_names)`. Default None; only TS/JS impls
    /// override.
    fn extract_re_export(&self, _node: &Node<'_>, _source: &[u8]) -> Option<(String, Vec<String>)> {
        None
    }

    /// v1.4 S-002 / AS-015..017 — TS-only: given an import-or-re-export
    /// node, return the subset of `imported_names` that carry the
    /// `type` modifier (whole-statement `import type { Foo }` OR
    /// per-name `import { Foo, type Bar }` OR re-export
    /// `export type { Foo } from 'mod'`). For aliased forms the LOCAL
    /// name is reported (mirroring `extract_imported_names` semantics).
    /// Default empty for non-TS langs (no equivalent type/value
    /// distinction at source level).
    fn extract_type_only_names(&self, _node: &Node<'_>, _source: &[u8]) -> Vec<String> {
        Vec::new()
    }
}

/// Pool of registered language specs. Indexed by `Lang`.
pub struct ParserPool {
    specs: Vec<Box<dyn LanguageSpec>>,
}

impl ParserPool {
    pub fn new() -> Self {
        Self {
            specs: vec![
                Box::new(langs::py::PythonLang),
                Box::new(langs::ts::TypeScriptLang),
                Box::new(langs::js::JavaScriptLang),
                Box::new(langs::go::GoLang),
                Box::new(langs::rs::RustLang),
                Box::new(langs::java::JavaLang),
                Box::new(langs::kotlin::KotlinLang),
                Box::new(langs::csharp::CSharpLang),
                Box::new(langs::ruby::RubyLang),
                Box::new(langs::php::PhpLang),
            ],
        }
    }

    pub fn registered_langs(&self) -> Vec<Lang> {
        self.specs.iter().map(|s| s.lang()).collect()
    }

    pub fn spec_for(&self, lang: Lang) -> Option<&dyn LanguageSpec> {
        self.specs
            .iter()
            .find(|s| s.lang() == lang)
            .map(|b| b.as_ref())
    }
}

impl Default for ParserPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `source` with the spec for `lang` and return flat list of symbols.
/// Uses a fresh [`tree_sitter::Parser`] per call — cheap enough for S-004.
/// Pool-level parser caching lands in S-005 (incremental indexing).
pub fn parse_source(lang: Lang, source: &[u8]) -> Result<Vec<ParsedSymbol>> {
    let pool = ParserPool::new();
    let spec = pool.spec_for(lang).ok_or_else(|| {
        ga_core::Error::Other(anyhow::anyhow!("no LanguageSpec registered for {lang:?}"))
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&spec.tree_sitter_lang())
        .map_err(|e| ga_core::Error::ParseError {
            file: "<source>".into(),
            lang: lang.as_str().into(),
            err: format!("set_language failed: {e}"),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ga_core::Error::ParseError {
            file: "<source>".into(),
            lang: lang.as_str().into(),
            err: "tree-sitter returned no tree".into(),
        })?;

    let mut out = Vec::new();
    walker::walk_tree(tree.root_node(), source, spec, None, &mut out);
    Ok(out)
}

/// Map tree-sitter node kind string → `SymbolKind`. Ported from
/// `rust-poc/src/main.rs:369-380`.
pub fn classify_kind(node_kind: &str) -> SymbolKind {
    if node_kind.contains("trait") {
        SymbolKind::Trait
    } else if node_kind.contains("interface") {
        SymbolKind::Interface
    } else if node_kind.contains("enum") {
        SymbolKind::Enum
    } else if node_kind.contains("struct") || node_kind.contains("impl") {
        SymbolKind::Struct
    } else if node_kind.contains("class") {
        SymbolKind::Class
    } else if node_kind.contains("method") {
        SymbolKind::Method
    } else if node_kind.contains("function") || node_kind.contains("arrow") {
        SymbolKind::Function
    } else {
        SymbolKind::Other
    }
}

/// Extract `name` field or fall back to per-lang parent-hop logic.
/// Ported from `rust-poc/src/main.rs:341-367`.
pub(crate) fn name_from_node(node: &Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(text) = name_node.utf8_text(source) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    // Arrow function inside variable_declarator: use the declarator's name.
    if node.kind() == "arrow_function" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "variable_declarator" {
                if let Some(name_node) = parent.child_by_field_name("name") {
                    if let Ok(text) = name_node.utf8_text(source) {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }
    // export_statement: peel one layer and retry.
    if node.kind() == "export_statement" {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if let Some(name) = name_from_node(&child, source) {
                    return Some(name);
                }
            }
        }
    }
    None
}
