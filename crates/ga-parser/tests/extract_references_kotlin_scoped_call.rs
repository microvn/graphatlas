//! Cross-lang sweep — Kotlin `Foo.bar()` (companion-object / object call)
//! receiver references.
//!
//! Mirror of LANG-2 (PHP `Class::method()`): Kotlin uses `Type.method()`
//! for companion-object calls (the equivalent of Java static dispatch).
//! The indexer must emit a REFERENCES edge to `Type` so `ga_callers Type`
//! surfaces every file that invokes any of `Type`'s companion methods.
//!
//! Heuristic: receiver identifier must be uppercase-first (Kotlin type
//! convention) and not a stdlib type.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::ParsedReference;

fn refs_of(src: &[u8]) -> Vec<ParsedReference> {
    extract_references(Lang::Kotlin, src).expect("extract_references Ok")
}

#[test]
fn scoped_call_emits_class_scope_reference() {
    // Regression: cross-lang sweep — `MyClass.staticFn(...)` call on a
    // Kotlin companion object should emit a REFERENCES edge to `MyClass`.
    let src = b"\
fun run() {
    MyClass.staticFn(42)
}
";
    let refs = refs_of(src);
    let r = refs.iter().find(|r| r.target_name == "MyClass");
    assert!(r.is_some(), "MyClass call-site ref not emitted: {refs:?}");
}

#[test]
fn this_super_scoped_call_not_emitted_as_class_ref() {
    // `this`, `super` are keywords — never user types. `it` is also a
    // Kotlin convention (lambda implicit) and must not become a ref.
    let src = b"\
class Foo {
    fun outer() {
        this.inner()
        super.bar()
    }
    fun inner() {}
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "this" | "super" | "it"))
        .collect();
    assert!(bad.is_empty(), "must not emit this/super/it refs: {bad:?}");
}

#[test]
fn stdlib_scoped_call_not_emitted() {
    // Math/System etc. are heavy; skip the stdlib types.
    let src = b"\
fun calc() {
    Math.max(1, 2)
    System.exit(0)
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "Math" | "System"))
        .collect();
    assert!(bad.is_empty(), "stdlib classes must not emit refs: {bad:?}");
}
