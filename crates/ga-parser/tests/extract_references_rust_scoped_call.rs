//! Cross-lang sweep — Rust `Foo::bar()` scope-receiver references.
//!
//! Mirror of LANG-2 (PHP `Class::method()`): when a Rust call uses
//! `Type::method(args)` syntax, the indexer must emit a REFERENCES edge to
//! `Type` (the scope receiver) so `ga_callers Type` surfaces every file that
//! invokes any of `Type`'s associated functions. Without this, the CALL
//! edge only carries the method name and querying the type returns empty.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! LANG-2 "cross-lang sweep" follow-up.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::ParsedReference;

fn refs_of(src: &[u8]) -> Vec<ParsedReference> {
    extract_references(Lang::Rust, src).expect("extract_references Ok")
}

#[test]
fn scoped_call_emits_type_scope_reference() {
    // Regression: cross-lang sweep — `MyStruct::new()` at line 2 should
    // emit a REFERENCES edge to `MyStruct` at the call site (in addition to
    // any type-position ref from `-> MyStruct` at line 1). Without this
    // emitter, files that ONLY invoke associated fns (no type position use)
    // return empty for `ga_callers MyStruct`.
    let src = b"\
fn run() {
    MyStruct::new(42)
}
";
    let refs = refs_of(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "MyStruct" && r.ref_site_line == 2)
        .unwrap_or_else(|| panic!("MyStruct call-site ref at line 2 not emitted: {refs:?}"));
    let _ = r;
}

#[test]
fn self_super_crate_scoped_call_not_emitted_as_type_ref() {
    // `self`, `super`, `crate`, `Self` are Rust path keywords — never user
    // types. They must not produce REFERENCES edges (would collide with
    // global symbols / confuse callers).
    let src = b"\
impl Foo {
    fn outer(&self) {
        self::inner();
        super::sibling();
        crate::root_fn();
        Self::associated();
    }
    fn inner() {}
    fn associated() {}
}
fn sibling() {}
mod root_fn { pub fn x() {} }
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "self" | "super" | "crate" | "Self"))
        .collect();
    assert!(
        bad.is_empty(),
        "Rust keywords must not become scope refs: {bad:?}"
    );
}

#[test]
fn nested_path_takes_trailing_segment() {
    // `std::process::Command::new(...)` — the scope receiver is `Command`
    // (trailing segment of the qualified path). Indexer treats `Command` as
    // the queryable type.
    let src = b"\
fn run() {
    std::process::Command::new(\"ls\")
}
";
    let refs = refs_of(src);
    let r = refs.iter().find(|r| r.target_name == "Command");
    assert!(r.is_some(), "Command not emitted: {refs:?}");
}
