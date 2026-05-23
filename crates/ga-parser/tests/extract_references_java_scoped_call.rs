//! Cross-lang sweep — Java `Foo.bar()` (class scope) receiver references.
//!
//! Mirror of LANG-2 (PHP `Class::method()`): when a Java call uses
//! `Class.method(args)` syntax (static call), the indexer must emit a
//! REFERENCES edge to `Class` so `ga_callers Class` surfaces every file that
//! invokes any of `Class`'s static methods.
//!
//! Heuristic: receiver identifier must be uppercase-first (Java class
//! convention) and not a stdlib type like `Math`, `System`, `Collections`,
//! `String`, etc. — those would explode the universe.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::ParsedReference;

fn refs_of(src: &[u8]) -> Vec<ParsedReference> {
    extract_references(Lang::Java, src).expect("extract_references Ok")
}

#[test]
fn scoped_call_emits_class_scope_reference() {
    // Regression: cross-lang sweep — `MyService.process(...)` static call
    // should emit a REFERENCES edge to `MyService` at the call site.
    let src = b"\
class Caller {
    void go() {
        MyService.process(42);
    }
}
";
    let refs = refs_of(src);
    let r = refs.iter().find(|r| r.target_name == "MyService");
    assert!(r.is_some(), "MyService call-site ref not emitted: {refs:?}");
}

#[test]
fn this_super_scoped_call_not_emitted_as_class_ref() {
    // `this`, `super` are keywords — never user types.
    let src = b"\
class Foo {
    void outer() {
        this.inner();
        super.bar();
    }
    void inner() {}
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "this" | "super"))
        .collect();
    assert!(bad.is_empty(), "must not emit this/super refs: {bad:?}");
}

#[test]
fn stdlib_scoped_call_not_emitted() {
    // Math/Collections/System are classes but used so heavily that emitting
    // refs would dominate the graph. Skip the well-known BCL types.
    let src = b"\
class Calc {
    void go() {
        Math.max(1, 2);
        System.out.println(\"x\");
        Collections.emptyList();
    }
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "Math" | "System" | "Collections"))
        .collect();
    assert!(bad.is_empty(), "stdlib classes must not emit refs: {bad:?}");
}
