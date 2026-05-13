//! v1.1-M4 S-002c — Kotlin SymbolAttribute + EnclosingScope (AS-007 / AS-008).
//!
//! AS-007 (Kotlin extension functions):
//!   Given: `fun String.isEmail(): Boolean = matches(emailRegex)`
//!   When: ParserPool indexes the file.
//!   Then: ParsedSymbol surfaces with
//!         - kind: SymbolKind::Function (Kotlin grammar emits
//!           `function_declaration` for both regular + extension fns;
//!           classify_kind has no ExtensionFunction variant)
//!         - enclosing: Some(EnclosingScope::ExtendedType("String"))
//!         - attributes: contains SymbolAttribute::ExtendedReceiver("String")
//!
//! AS-008 (Kotlin suspend functions):
//!   Given: `suspend fun fetchUser(id: Int): User`
//!   When: ParserPool indexes the file.
//!   Then: ParsedSymbol surfaces with
//!         - kind: SymbolKind::Function
//!         - attributes: contains SymbolAttribute::Suspend
//!
//! S2 spec-update note: the spec sub-doc currently says
//! `kind: "extension_function"` / `kind: "suspend_function"` — those are
//! aspirational shapes from before S-005a D5 reified SymbolAttribute /
//! EnclosingScope. The shipped contract uses those D5 mechanisms.

use ga_core::{Lang, SymbolKind};
use ga_parser::{parse_source, EnclosingScope, SymbolAttribute};

#[test]
fn top_level_extension_function_records_extended_type_enclosing() {
    let src = b"fun String.isEmail(): Boolean = matches(emailRegex)\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let is_email = syms
        .iter()
        .find(|s| s.name == "isEmail")
        .unwrap_or_else(|| panic!("isEmail symbol missing: {syms:?}"));
    assert_eq!(is_email.kind, SymbolKind::Function);
    match &is_email.enclosing {
        Some(EnclosingScope::ExtendedType(name)) => {
            assert_eq!(
                name, "String",
                "extension fn enclosing receiver must be `String`, got `{name}`"
            );
        }
        other => panic!(
            "AS-007: top-level `fun String.isEmail()` must have enclosing = \
             ExtendedType(\"String\"), got {other:?} (full sym: {is_email:?})"
        ),
    }
}

#[test]
fn extension_function_carries_extended_receiver_attribute() {
    let src = b"fun List<Int>.sumOrZero(): Int = if (isEmpty()) 0 else sum()\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let sum_or_zero = syms
        .iter()
        .find(|s| s.name == "sumOrZero")
        .unwrap_or_else(|| panic!("sumOrZero missing: {syms:?}"));
    let has_receiver = sum_or_zero
        .attributes
        .iter()
        .any(|a| matches!(a, SymbolAttribute::ExtendedReceiver(t) if t == "List"));
    assert!(
        has_receiver,
        "AS-007: extension fn `List<Int>.sumOrZero()` must carry \
         ExtendedReceiver(\"List\") attribute (raw type, generic args stripped); \
         got attrs={:?}",
        sum_or_zero.attributes
    );
}

#[test]
fn non_extension_function_has_no_extended_receiver_attribute() {
    let src = b"fun helper(): Int = 0\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let helper = syms
        .iter()
        .find(|s| s.name == "helper")
        .unwrap_or_else(|| panic!("helper missing: {syms:?}"));
    let has_receiver = helper
        .attributes
        .iter()
        .any(|a| matches!(a, SymbolAttribute::ExtendedReceiver(_)));
    assert!(
        !has_receiver,
        "regular fn must NOT carry ExtendedReceiver attribute, got: {:?}",
        helper.attributes
    );
    assert!(
        !matches!(helper.enclosing, Some(EnclosingScope::ExtendedType(_))),
        "regular fn must NOT have ExtendedType enclosing, got: {:?}",
        helper.enclosing
    );
}

#[test]
fn suspend_function_carries_suspend_attribute() {
    let src = b"suspend fun fetchUser(id: Int): User { return repo.get(id) }\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let fetch = syms
        .iter()
        .find(|s| s.name == "fetchUser")
        .unwrap_or_else(|| panic!("fetchUser missing: {syms:?}"));
    assert!(
        fetch.attributes.contains(&SymbolAttribute::Suspend),
        "AS-008: `suspend fun fetchUser` must carry SymbolAttribute::Suspend, got: {:?}",
        fetch.attributes
    );
}

#[test]
fn non_suspend_function_does_not_carry_suspend_attribute() {
    let src = b"fun greet() {}\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let greet = syms
        .iter()
        .find(|s| s.name == "greet")
        .unwrap_or_else(|| panic!("greet missing: {syms:?}"));
    assert!(
        !greet.attributes.contains(&SymbolAttribute::Suspend),
        "regular fn must NOT carry Suspend attribute, got: {:?}",
        greet.attributes
    );
}

#[test]
fn suspend_extension_function_carries_both_attributes() {
    // Realistic Android pattern: suspend fn extending a flow / channel.
    let src = b"suspend fun Flow<Int>.collectAll(): List<Int> = toList()\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let collect_all = syms
        .iter()
        .find(|s| s.name == "collectAll")
        .unwrap_or_else(|| panic!("collectAll missing: {syms:?}"));
    assert!(
        collect_all.attributes.contains(&SymbolAttribute::Suspend),
        "suspend extension fn must carry Suspend: {:?}",
        collect_all.attributes
    );
    let has_receiver = collect_all
        .attributes
        .iter()
        .any(|a| matches!(a, SymbolAttribute::ExtendedReceiver(t) if t == "Flow"));
    assert!(
        has_receiver,
        "suspend extension fn must also carry ExtendedReceiver(\"Flow\"): {:?}",
        collect_all.attributes
    );
}

#[test]
fn class_method_inside_class_keeps_class_enclosing() {
    // Sanity guard: AS-007 override must NOT clobber non-extension methods
    // inside a class. The walker's existing Class-tracking must still win
    // when no ExtendedReceiver is detected.
    let src = b"class Foo { fun bar() = baz() }\n";
    let syms = parse_source(Lang::Kotlin, src).expect("parse_source Ok");
    let bar = syms
        .iter()
        .find(|s| s.name == "bar")
        .unwrap_or_else(|| panic!("bar missing: {syms:?}"));
    match &bar.enclosing {
        Some(EnclosingScope::Class(name)) => assert_eq!(name, "Foo"),
        other => {
            panic!("non-extension method inside class must have Class enclosing, got: {other:?}")
        }
    }
}
