//! v1.1-M4 S-004c — Ruby symbol extraction (AS-014 metaprogramming).
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-014):
//!   `define_method(:foo) { ... }` emits a synthesized symbol named `foo`
//!   with `confidence: 0.6` per Tools-C11. Tree-sitter cannot statically
//!   resolve string-named symbols — the lower confidence reflects the
//!   honest limitation of parse-time analysis.
//!
//! Implementation: `RubyLang::extract_synthetic_symbol` is invoked by the
//! walker on every node. Returns Some(...) only for `call` nodes whose
//! first child is the bare identifier `define_method` (no receiver).

use ga_core::{Lang, SymbolKind};
use ga_parser::{parse_source, ParsedSymbol, SymbolAttribute};

fn syms(src: &str) -> Vec<ParsedSymbol> {
    parse_source(Lang::Ruby, src.as_bytes()).expect("parse_source Ok")
}

fn find<'a>(out: &'a [ParsedSymbol], name: &str) -> Option<&'a ParsedSymbol> {
    out.iter().find(|s| s.name == name)
}

// ─────────────────────────────────────────────────────────────────────────
// AS-014 — define_method synthesizes a symbol with confidence 0.6
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn define_method_with_block_emits_symbol_with_reduced_confidence() {
    let src = "\
class Repo\n\
  define_method(:dyn) { puts 'x' }\n\
end\n";
    let out = syms(src);
    let dyn_sym =
        find(&out, "dyn").expect("define_method(:dyn) must surface as a Symbol named `dyn`");
    assert!(
        (dyn_sym.confidence - 0.6).abs() < 1e-6,
        "AS-014 Tools-C11: confidence must be 0.6 (got {})",
        dyn_sym.confidence
    );
    assert_eq!(dyn_sym.kind, SymbolKind::Method);
}

#[test]
fn define_method_carries_decorator_attribute() {
    // Marks the symbol's provenance for downstream consumers (ga_symbols
    // meta.warning per AS-014 Then clause).
    let src = "\
class Repo\n\
  define_method(:dyn) { 1 }\n\
end\n";
    let out = syms(src);
    let dyn_sym = find(&out, "dyn").unwrap();
    assert!(
        dyn_sym.attributes.iter().any(
            |a| matches!(a, SymbolAttribute::Decorator { name, .. } if name == "define_method")
        ),
        "expected SymbolAttribute::Decorator(\"define_method\"), got {:?}",
        dyn_sym.attributes
    );
}

#[test]
fn define_method_with_do_end_block_form_also_emits() {
    // Ruby allows both `{ ... }` and `do ... end` block forms — both must
    // surface the same way.
    let src = "\
class Repo\n\
  define_method :bare do |x|\n\
    inner(x)\n\
  end\n\
end\n";
    let out = syms(src);
    let bare = find(&out, "bare").expect("do/end block form must also emit");
    assert!((bare.confidence - 0.6).abs() < 1e-6);
}

#[test]
fn statically_defined_methods_keep_confidence_one() {
    // Sanity: regular `def foo` retains confidence 1.0; only synthetic
    // emission drops to 0.6. AS-014 must not pollute the canonical path.
    let src = "\
class Repo\n\
  def static_method\n\
    1\n\
  end\n\
  define_method(:dyn) { 2 }\n\
end\n";
    let out = syms(src);
    let static_sym = find(&out, "static_method").unwrap();
    assert!(
        (static_sym.confidence - 1.0).abs() < 1e-6,
        "static def must retain confidence 1.0, got {}",
        static_sym.confidence
    );
    let dyn_sym = find(&out, "dyn").unwrap();
    assert!((dyn_sym.confidence - 0.6).abs() < 1e-6);
}

#[test]
fn define_method_with_string_literal_argument_does_not_emit() {
    // `define_method("foo")` — string-literal arg is rare but legal. We
    // only synthesize from `simple_symbol` form (the idiomatic Ruby
    // pattern). String form is a known limitation; better to under-emit
    // than fabricate a symbol from arbitrary text.
    let src = "\
class Repo\n\
  define_method(\"foo\") { 1 }\n\
end\n";
    let out = syms(src);
    assert!(
        find(&out, "foo").is_none(),
        "string-arg define_method should NOT emit a synthetic symbol"
    );
}

#[test]
fn receiver_form_define_method_does_not_emit_synthetic_symbol() {
    // `obj.define_method(:foo) { ... }` is a runtime call on an instance;
    // it does NOT statically define a method on the enclosing class.
    // The synthetic-symbol hook must skip receiver-form calls.
    let src = "\
class Repo\n\
  obj.define_method(:foo) { 1 }\n\
end\n";
    let out = syms(src);
    assert!(
        find(&out, "foo").is_none(),
        "receiver-form define_method must not synthesize a class-level symbol"
    );
}

#[test]
fn define_method_outside_class_still_emits() {
    // Ruby allows `define_method` at the top level (defines on Object).
    // The parser layer doesn't enforce class-context — only checks syntax.
    let src = "define_method(:top_level) { 1 }\n";
    let out = syms(src);
    assert!(
        find(&out, "top_level").is_some(),
        "top-level define_method should still surface a synthetic symbol"
    );
}

#[test]
fn malformed_define_method_does_not_panic() {
    // R12 panic-safety on garbage input.
    let garbage: &[u8] = b"define_method(:foo \xff\xfe { ";
    let result = std::panic::catch_unwind(|| parse_source(Lang::Ruby, garbage));
    assert!(result.is_ok(), "parse_source panicked on malformed input");
}

// ─────────────────────────────────────────────────────────────────────────
// Static methods continue to work (regression coverage)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn static_methods_emit_with_full_confidence() {
    let src = "\
class User\n\
  def authenticate(pw)\n\
    check(pw)\n\
  end\n\
  def self.find(id)\n\
    Base.lookup(id)\n\
  end\n\
end\n";
    let out = syms(src);
    let auth = find(&out, "authenticate").expect("def authenticate");
    assert_eq!(auth.kind, SymbolKind::Method);
    assert!((auth.confidence - 1.0).abs() < 1e-6);
    let find_sym = find(&out, "find").expect("def self.find (singleton_method)");
    assert!((find_sym.confidence - 1.0).abs() < 1e-6);
}

#[test]
fn class_and_module_decls_emit_symbols() {
    let src = "\
module App\n\
  class User < Base\n\
  end\n\
end\n";
    let out = syms(src);
    assert!(find(&out, "App").is_some(), "module App");
    assert!(find(&out, "User").is_some(), "class User");
}
