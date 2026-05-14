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
