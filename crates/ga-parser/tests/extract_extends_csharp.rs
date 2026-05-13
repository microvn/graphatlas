//! v1.1-M4 S-003c — C# EXTENDS extraction (AS-010).
//!
//! Lang-C1 atomic UC gate: each base type after `:` (`base_list`) emits
//! one EXTENDS edge. C# uses unified `:` for both class extension and
//! interface implementation (idiomatic: first item is class, rest are
//! interfaces — but parser doesn't distinguish; both surface as bases).
//!
//! Examples:
//!   - `class Admin : User, IPrintable` → bases=[User, IPrintable]
//!   - `interface IFoo : IBar, IBaz`    → bases=[IBar, IBaz]
//!   - `class Box : Container<int>`     → bases=[Container] (generic raw)
//!   - `class Q : System.User`          → bases=[User] (qualified strip)

use ga_core::Lang;
use ga_parser::extract_extends;

#[test]
fn class_with_class_base_emits_extends_edge() {
    let src = b"class User {}\nclass Admin : User {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let admin: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        admin.contains(&"User"),
        "Admin : User must emit EXTENDS(Admin → User), got: {edges:?}"
    );
}

#[test]
fn class_with_interface_emits_extends_edge() {
    let src = b"interface IPrintable {}\nclass Service : IPrintable {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Service")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"IPrintable"),
        "Service : IPrintable must emit EXTENDS edge, got: {edges:?}"
    );
}

#[test]
fn class_with_class_and_multiple_interfaces_emits_one_per_base() {
    // AS-010 canonical example.
    let src = b"class Admin : User, IPrintable, ICloneable {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    for expected in &["User", "IPrintable", "ICloneable"] {
        assert!(
            bases.contains(expected),
            "Admin must extend `{expected}`; got bases={bases:?}"
        );
    }
    assert!(bases.len() >= 3, "expected ≥3 EXTENDS edges; got {edges:?}");
}

#[test]
fn interface_extending_interfaces_emits_edge_per_base() {
    let src = b"interface IFoo : IBar, IBaz {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "IFoo")
        .map(|e| e.base_name.as_str())
        .collect();
    for expected in &["IBar", "IBaz"] {
        assert!(
            bases.contains(expected),
            "IFoo must extend `{expected}`: {edges:?}"
        );
    }
}

#[test]
fn qualified_base_strips_namespace_prefix() {
    let src = b"class Q : System.User {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Q")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"User"),
        "qualified `System.User` must surface trailing `User`: {edges:?}"
    );
    assert!(
        !bases.contains(&"System"),
        "must not leak namespace prefix into base: {edges:?}"
    );
}

#[test]
fn generic_base_emits_raw_type_name() {
    let src = b"class Box : Container<int> {}\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Box")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"Container"),
        "generic `Container<int>` must surface raw `Container`: {edges:?}"
    );
}

#[test]
fn class_without_base_emits_no_edges() {
    let src = b"class Lone { public int X = 0; }\n";
    let edges = extract_extends(Lang::CSharp, src).expect("extract_extends Ok");
    let lone: Vec<_> = edges.iter().filter(|e| e.class_name == "Lone").collect();
    assert!(
        lone.is_empty(),
        "no `:` declaration → no EXTENDS edges, got: {lone:?}"
    );
}

#[test]
fn empty_source_returns_ok_empty() {
    let edges = extract_extends(Lang::CSharp, b"").expect("extract_extends Ok on empty");
    assert!(edges.is_empty());
}

#[test]
fn malformed_csharp_source_does_not_panic_in_extends_walker() {
    let garbage: &[u8] = b"class }}}{ : <<< abandon !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_extends(Lang::CSharp, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_extends panicked on garbage C# input"
    );
}
