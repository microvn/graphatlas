//! Cross-lang sweep — Python `Foo.bar()` (class scope) receiver references.
//!
//! Mirror of LANG-2 (PHP `Class::method()`): when a Python call uses
//! `Class.method(args)` syntax, the indexer must emit a REFERENCES edge to
//! `Class` (the scope receiver) so `ga_callers Class` surfaces every file
//! that invokes any of `Class`'s classmethods/staticmethods.
//!
//! Python lacks a dedicated `::` syntax, so the receiver kind is
//! heuristically distinguished from instance-method calls (`foo.bar()`) by
//! PascalCase convention: first char of the receiver must be uppercase.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! LANG-2 "cross-lang sweep" follow-up.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::ParsedReference;

fn refs_of(src: &[u8]) -> Vec<ParsedReference> {
    extract_references(Lang::Python, src).expect("extract_references Ok")
}

#[test]
fn scoped_call_emits_class_scope_reference() {
    // Regression: cross-lang sweep — `MyService.process(...)` should emit a
    // REFERENCES edge to `MyService` at the call site. Without this, files
    // that ONLY invoke classmethods (no other type-position use of the class)
    // return empty for `ga_callers MyService`.
    let src = b"\
def run():
    MyService.process(42)
";
    let refs = refs_of(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "MyService" && r.ref_site_line == 2);
    assert!(
        r.is_some(),
        "MyService call-site ref at line 2 not emitted: {refs:?}"
    );
}

#[test]
fn self_cls_super_scoped_call_not_emitted_as_class_ref() {
    // `self`, `cls`, `super` are Python-convention identifiers — never user
    // types. They must not produce REFERENCES edges. Also instance calls
    // (`obj.method()`) — lowercase receiver — must not emit.
    let src = b"\
class Foo:
    def outer(self):
        self.inner()
        cls.classmethod()
        super().bar()

    @classmethod
    def klass(cls):
        cls.inner()

    def instance_call(self):
        obj.method()
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "self" | "cls" | "super" | "obj"))
        .collect();
    assert!(
        bad.is_empty(),
        "must not emit self/cls/super/obj refs: {bad:?}"
    );
}

#[test]
fn builtin_scoped_call_not_emitted() {
    // Builtins like `int.from_bytes()` / `str.join()` would explode the
    // universe — skip the well-known stdlib types.
    let src = b"\
def run():
    int.from_bytes(b'x', 'big')
    str.join(',', ['a', 'b'])
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "int" | "str"))
        .collect();
    assert!(
        bad.is_empty(),
        "builtins must not emit class-scope refs: {bad:?}"
    );
}
