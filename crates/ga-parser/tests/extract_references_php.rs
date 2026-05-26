//! v1.2-php S-001 AS-004 — PHP DI annotation refs.
//!
//! `#[Required]` on a property → ParsedReference with RefKind::AnnotatedFieldType
//! to the property type. `#[ORM\Column]` does NOT emit (carve-out Lang-C7).
//! Multi-attribute on same property = 1 ref, not N (dedup).

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::RefKind;

fn refs_of(src: &[u8]) -> Vec<ga_parser::references::ParsedReference> {
    extract_references(Lang::Php, src).expect("extract_references Ok")
}

// LANG-2 regression suite (2026-05-22) — `Class::method()` must emit a
// REFERENCES edge to the class scope so `ga_callers Class` surfaces files
// that invoke any of the class's static methods. Without this, the indexer
// drops the class scope and only stores the unqualified method name in the
// CALL edge — querying the class returns empty.
//
// Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md LANG-2.

#[test]
fn scoped_call_emits_class_scope_reference() {
    // Regression: LANG-2 — `RsSession::sign(...)` should emit a REFERENCES
    // edge to `RsSession` (class scope) in addition to the existing CALL
    // edge to `sign` (method).
    let src = b"\
<?php
class Caller {
    public function go(): void {
        RsSession::sign(['k' => 'v'], 'secret');
    }
}
";
    let refs = refs_of(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "RsSession")
        .unwrap_or_else(|| panic!("RsSession class scope ref not emitted: {refs:?}"));
    // Line 4 = `RsSession::sign(...)` call.
    assert_eq!(r.ref_site_line, 4, "ref line: {r:?}");
}

#[test]
fn self_parent_static_scoped_call_not_emitted_as_class_ref() {
    // `self::`, `parent::`, `static::` are language keywords, not user types.
    // They must NOT produce REFERENCES edges to literal "self"/"parent"/"static"
    // — those names would collide with global symbols.
    let src = b"\
<?php
class Foo {
    public function inner(): void {
        self::bar();
        parent::baz();
        static::qux();
    }
}
";
    let refs = refs_of(src);
    let bad: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.target_name.as_str(), "self" | "parent" | "static"))
        .collect();
    assert!(
        bad.is_empty(),
        "should not emit self/parent/static refs: {bad:?}"
    );
}

#[test]
fn required_attribute_on_property_emits_annotated_field_type_ref() {
    let src = b"\
<?php
class UserController {
    #[Required]
    public UserRepository $repo;
}
";
    let refs = refs_of(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "UserRepository")
        .unwrap_or_else(|| panic!("UserRepository ref not emitted: {refs:?}"));
    assert!(
        matches!(r.ref_kind, RefKind::AnnotatedFieldType),
        "ref_kind must be AnnotatedFieldType, got {:?}",
        r.ref_kind
    );
}

#[test]
fn orm_column_attribute_does_not_emit_annotated_field_type_ref() {
    // Lang-C7 carve-out: Doctrine ORM column attributes route via decorator,
    // NOT AnnotatedFieldType (which is DI intent only).
    let src = b"\
<?php
class Entity {
    #[ORM\\Column(type: 'string')]
    public string $name;
}
";
    let refs = refs_of(src);
    let has_annotated_field = refs
        .iter()
        .any(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType) && r.target_name == "string");
    assert!(
        !has_annotated_field,
        "Doctrine ORM\\Column must NOT emit AnnotatedFieldType: {refs:?}"
    );
}

#[test]
fn property_with_multiple_di_attributes_emits_one_ref() {
    // AS-004 dedup: #[Required] #[Lazy] public Logger $logger → 1 ref to Logger.
    let src = b"\
<?php
class App {
    #[Required]
    #[Lazy]
    public Logger $logger;
}
";
    let refs = refs_of(src);
    let logger_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.target_name == "Logger" && matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert_eq!(
        logger_refs.len(),
        1,
        "multi-attribute property must emit ONE ref, not {}: {refs:?}",
        logger_refs.len()
    );
}
