//! Cross-lang sweep — C# `Foo.Bar()` (class scope) receiver references.
//!
//! Mirror of LANG-2 (PHP `Class::method()`): when a C# call uses
//! `Class.Method(args)` syntax (static call), the indexer must emit a
//! REFERENCES edge to `Class` so `ga_callers Class` surfaces every file
//! that invokes any of `Class`'s static methods.
//!
//! Heuristic: receiver identifier must be uppercase-first (C# class
//! convention) and not a stdlib type like `Math`, `Console`, `String`, etc.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::ParsedReference;

fn refs_of(src: &[u8]) -> Vec<ParsedReference> {
    extract_references(Lang::CSharp, src).expect("extract_references Ok")
}

#[test]
fn scoped_call_emits_class_scope_reference() {
    // Regression: cross-lang sweep — `MyController.Index(...)` static call
    // should emit a REFERENCES edge to `MyController` at the call site.
    let src = b"\
class Caller {
    void Go() {
        MyController.Index(42);
    }
}
";
    let refs = refs_of(src);
    let r = refs.iter().find(|r| r.target_name == "MyController");
    assert!(
        r.is_some(),
        "MyController call-site ref not emitted: {refs:?}"
    );
}

#[test]
fn this_base_scoped_call_not_emitted_as_class_ref() {
    // `this`, `base` are keywords — never user types.
    let src = b"\
class Foo {
    void Outer() {
        this.Inner();
        base.Bar();
    }
    void Inner() {}
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "this" | "base"))
        .collect();
    assert!(bad.is_empty(), "must not emit this/base refs: {bad:?}");
}

#[test]
fn stdlib_scoped_call_not_emitted() {
    // Console/Math/etc. are classes but used so heavily that emitting refs
    // would dominate the graph.
    let src = b"\
class Calc {
    void Go() {
        Console.WriteLine(\"x\");
        Math.Max(1, 2);
    }
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "Console" | "Math"))
        .collect();
    assert!(bad.is_empty(), "stdlib classes must not emit refs: {bad:?}");
}
