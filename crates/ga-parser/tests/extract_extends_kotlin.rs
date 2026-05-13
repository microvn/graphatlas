//! v1.1-M4 S-002c — Kotlin EXTENDS extraction (AS-002-equiv).
//!
//! Lang-C1 atomic UC gate: each base type after `:` (delegation_specifier)
//! emits one EXTENDS edge. Kotlin uses unified `:` syntax for both
//! superclass extension (with `()` constructor invocation) and interface
//! implementation (without `()`). Tree-sitter exposes both as
//! `delegation_specifier` children of `delegation_specifiers`.
//!
//! Examples:
//!   - `class Admin : User(), Printable` → bases=[User, Printable]
//!   - `interface Z : A, B`               → bases=[A, B]
//!   - `class Box<T> : Container<T>()`    → bases=[Container] (generic raw)
//!   - `class Q : com.example.Base()`     → bases=[Base] (qualified strip)

use ga_core::Lang;
use ga_parser::extract_extends;

#[test]
fn class_with_constructor_invocation_emits_extends_edge() {
    let src = b"open class User\nclass Admin : User()\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let admin_bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        admin_bases.contains(&"User"),
        "Admin : User() must emit EXTENDS(Admin → User), got: {edges:?}"
    );
}

#[test]
fn class_with_interface_implements_emits_extends_edge() {
    let src = b"interface Printable\nclass Service : Printable\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let service_bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Service")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        service_bases.contains(&"Printable"),
        "Service : Printable must emit EXTENDS(Service → Printable), got: {edges:?}"
    );
}

#[test]
fn class_with_multiple_supertypes_emits_one_edge_per_base() {
    let src = b"class Admin : User(), Printable, Cloneable\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    for expected in &["User", "Printable", "Cloneable"] {
        assert!(
            bases.contains(expected),
            "Admin must extend `{expected}`; got bases={bases:?}"
        );
    }
    assert!(bases.len() >= 3, "expected ≥3 EXTENDS edges; got {edges:?}");
}

#[test]
fn qualified_supertype_strips_module_prefix() {
    // `class Q : com.example.Base()` → base="Base" (last identifier).
    let src = b"class Q : com.example.Base()\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Q")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"Base"),
        "qualified `com.example.Base()` must surface `Base` (last segment): {edges:?}"
    );
    assert!(
        !bases.contains(&"com"),
        "must not leak module prefix into base: {edges:?}"
    );
}

#[test]
fn generic_supertype_emits_raw_type_name() {
    // `class Box<T> : Container<T>()` → base="Container".
    let src = b"class Box<T> : Container<T>()\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Box")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"Container"),
        "generic `Container<T>()` must surface raw `Container`: {edges:?}"
    );
}

#[test]
fn class_without_supertype_emits_no_edges() {
    let src = b"class Lone { val x: Int = 0 }\n";
    let edges = extract_extends(Lang::Kotlin, src).expect("extract_extends Ok");
    let lone_edges: Vec<_> = edges.iter().filter(|e| e.class_name == "Lone").collect();
    assert!(
        lone_edges.is_empty(),
        "no `:` supertype declaration → no EXTENDS edges, got: {lone_edges:?}"
    );
}

#[test]
fn empty_source_returns_ok_empty() {
    let edges = extract_extends(Lang::Kotlin, b"").expect("extract_extends Ok on empty");
    assert!(edges.is_empty());
}

#[test]
fn malformed_kotlin_source_does_not_panic_in_extends_walker() {
    let garbage: &[u8] = b"class }}}{ : <<< abandon !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_extends(Lang::Kotlin, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_extends panicked on garbage Kotlin input"
    );
}
