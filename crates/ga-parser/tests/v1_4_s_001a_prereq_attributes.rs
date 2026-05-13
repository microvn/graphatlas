//! v1.4 S-001a-prereq — Rust + Python + TypeScript parser support for
//! `SymbolAttribute::Override`.
//!
//! Spec: graphatlas-v1.4-data-model.md S-001a-prereq, AS-007 / AS-008 /
//! AS-020 / AS-021. v1.3 Gap 4 closed Java / Kotlin / C# only;
//! /mf-challenge C2 + /mf-voices added the TS gap. This test file shipped
//! with the parser additions that close the 3-language hole.

use ga_core::Lang;
use ga_parser::{parse_source, SymbolAttribute};

fn has_override(symbols: &[ga_parser::ParsedSymbol], name: &str) -> bool {
    symbols
        .iter()
        .find(|s| s.name == name)
        .map(|s| {
            s.attributes
                .iter()
                .any(|a| matches!(a, SymbolAttribute::Override))
        })
        .unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────────
// AS-007 — Rust trait-method impl emits Override
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn rust_trait_impl_method_emits_override() {
    let src = br#"
trait Animal {
    fn speak(&self);
}

struct Dog;

impl Animal for Dog {
    fn speak(&self) {
        println!("woof");
    }
}
"#;
    let symbols = parse_source(Lang::Rust, src).unwrap();
    // Dog::speak is the trait-impl method — must have Override.
    assert!(
        has_override(&symbols, "speak"),
        "Rust trait-impl `fn speak` must emit SymbolAttribute::Override; got {:?}",
        symbols
            .iter()
            .find(|s| s.name == "speak")
            .map(|s| &s.attributes)
    );
}

#[test]
fn rust_inherent_impl_method_does_not_emit_override() {
    // AS-021 negative: inherent `impl Dog { fn speak() }` (no trait) MUST
    // NOT emit Override. Tests the parser's discrimination between
    // trait-impl and inherent-impl.
    let src = br#"
struct Dog;

impl Dog {
    fn speak(&self) {}
    fn bark(&self) {}
}
"#;
    let symbols = parse_source(Lang::Rust, src).unwrap();
    assert!(
        !has_override(&symbols, "speak"),
        "Rust inherent impl `fn speak` must NOT emit Override (no trait context)"
    );
    assert!(
        !has_override(&symbols, "bark"),
        "Rust inherent impl `fn bark` must NOT emit Override (no trait context)"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-008 — Python @override and @typing.override decorators emit Override
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn python_bare_override_decorator_emits_override() {
    let src = br#"
from typing import override

class Animal:
    def speak(self): pass

class Dog(Animal):
    @override
    def speak(self): pass
"#;
    let symbols = parse_source(Lang::Python, src).unwrap();
    // Dog.speak (with @override) must have Override; Animal.speak must not.
    let dog_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line >= 7)
        .expect("Dog.speak symbol not found");
    assert!(
        dog_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "Python `@override` must emit Override; attributes: {:?}",
        dog_speak.attributes
    );
    let animal_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line < 7)
        .expect("Animal.speak symbol not found");
    assert!(
        !animal_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "Animal.speak (no decorator) must NOT emit Override"
    );
}

#[test]
fn python_typing_override_qualified_emits_override() {
    // PEP 698 qualified form: `@typing.override`. Should be recognised as
    // Override the same way as bare `@override`.
    let src = br#"
import typing

class Animal:
    def speak(self): pass

class Dog(Animal):
    @typing.override
    def speak(self): pass
"#;
    let symbols = parse_source(Lang::Python, src).unwrap();
    let dog_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line >= 7)
        .expect("Dog.speak symbol not found");
    assert!(
        dog_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "Python `@typing.override` must emit Override; attributes: {:?}",
        dog_speak.attributes
    );
}

#[test]
fn python_undecorated_subclass_method_does_not_emit_override() {
    // AS-021 negative: subclass method WITHOUT @override decorator must
    // NOT emit Override (Python doesn't have a syntactic override marker
    // outside the decorator).
    let src = br#"
class Animal:
    def speak(self): pass

class Dog(Animal):
    def speak(self): pass
"#;
    let symbols = parse_source(Lang::Python, src).unwrap();
    let dog_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line >= 5)
        .expect("Dog.speak symbol not found");
    assert!(
        !dog_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "Python undecorated subclass method must NOT emit Override"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-020 — TypeScript 4.3+ override modifier emits Override
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn typescript_override_modifier_emits_override() {
    let src = br#"
class Animal {
    speak(): void {}
}

class Dog extends Animal {
    override speak(): void {}
}
"#;
    let symbols = parse_source(Lang::TypeScript, src).unwrap();
    // Dog.speak (with override modifier) must have Override.
    let dog_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line >= 6)
        .expect("Dog.speak symbol not found");
    assert!(
        dog_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "TS `override speak()` must emit Override; attributes: {:?}",
        dog_speak.attributes
    );
}

#[test]
fn typescript_no_override_modifier_does_not_emit_override() {
    // AS-021 negative: subclass method WITHOUT override modifier must
    // NOT emit Override (legal in TS without --noImplicitOverride).
    let src = br#"
class Animal {
    speak(): void {}
}

class Dog extends Animal {
    speak(): void {}
}
"#;
    let symbols = parse_source(Lang::TypeScript, src).unwrap();
    let dog_speak = symbols
        .iter()
        .find(|s| s.name == "speak" && s.line >= 6)
        .expect("Dog.speak symbol not found");
    assert!(
        !dog_speak
            .attributes
            .iter()
            .any(|a| matches!(a, SymbolAttribute::Override)),
        "TS subclass method without override modifier must NOT emit Override"
    );
}
