//! v1.1-M4 S-002c — Kotlin annotated-property REFERENCES (AS-004-equiv).
//!
//! Lang-C1 atomic UC gate. Lang-C7 generalizes Java's @Autowired contract
//! to Kotlin: a property carrying any annotation (`@Inject`, `@Composable`,
//! `@field:Inject`, ...) emits one ParsedReference whose target_name is
//! the property's declared type and ref_kind is AnnotatedFieldType.
//!
//! Tree-sitter shape:
//!   property_declaration
//!     ├── modifiers
//!     │     └── annotation { user_type: identifier "Inject" }
//!     ├── var | val
//!     └── variable_declaration
//!           ├── identifier (property name)
//!           └── user_type   (declared type — what we emit reference TO)
//!
//! Mirrors Java field_declaration emit-once-per-property semantics:
//! multiple annotations on the same property → ONE ref to its type.

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::RefKind;

fn refs_in(src: &[u8]) -> Vec<ga_parser::references::ParsedReference> {
    extract_references(Lang::Kotlin, src).expect("extract_references Ok")
}

#[test]
fn annotated_property_emits_reference_to_type() {
    // AS-004-equiv canonical: @Inject lateinit var repo: UserRepository
    // → ParsedReference{target_name="UserRepository", ref_kind=AnnotatedFieldType}.
    let src = b"\
class Service {\n\
    @Inject lateinit var repo: UserRepository\n\
}\n";
    let refs = refs_in(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "UserRepository")
        .unwrap_or_else(|| panic!("no reference to UserRepository: {refs:?}"));
    assert!(
        matches!(r.ref_kind, RefKind::AnnotatedFieldType),
        "Lang-C7: annotated property ref must use AnnotatedFieldType, got {:?}",
        r.ref_kind
    );
}

#[test]
fn property_without_annotation_emits_no_reference() {
    // Plain property — no annotation → no AnnotatedFieldType ref.
    let src = b"\
class Service {\n\
    val repo: UserRepository = UserRepository()\n\
}\n";
    let refs = refs_in(src);
    let annotated: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert!(
        annotated.is_empty(),
        "non-annotated property must NOT emit AnnotatedFieldType ref: {annotated:?}"
    );
}

#[test]
fn multiple_annotations_on_same_property_emit_single_ref() {
    // `@Inject @Lazy lateinit var x: UserRepository` → ONE ref, not two.
    let src = b"\
class Service {\n\
    @Inject @Lazy lateinit var repo: UserRepository\n\
}\n";
    let refs = refs_in(src);
    let urefs: Vec<_> = refs
        .iter()
        .filter(|r| r.target_name == "UserRepository")
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert_eq!(
        urefs.len(),
        1,
        "multi-annotation must dedup to one ref per property: {urefs:?}"
    );
}

#[test]
fn qualified_property_type_strips_to_trailing_identifier() {
    // `var repo: com.example.UserRepository` → target_name="UserRepository".
    let src = b"\
class Service {\n\
    @Inject lateinit var repo: com.example.UserRepository\n\
}\n";
    let refs = refs_in(src);
    assert!(
        refs.iter().any(|r| r.target_name == "UserRepository"
            && matches!(r.ref_kind, RefKind::AnnotatedFieldType)),
        "qualified type must surface trailing `UserRepository`: {refs:?}"
    );
    assert!(
        !refs
            .iter()
            .any(|r| r.target_name == "com.example.UserRepository"),
        "must NOT leak FQN into target_name: {refs:?}"
    );
}

#[test]
fn generic_property_type_emits_raw_type_name() {
    // `@Inject lateinit var users: List<UserRepository>` → "List" (raw).
    let src = b"\
class Service {\n\
    @Inject lateinit var users: List<UserRepository>\n\
}\n";
    let refs = refs_in(src);
    assert!(
        refs.iter()
            .any(|r| r.target_name == "List" && matches!(r.ref_kind, RefKind::AnnotatedFieldType)),
        "generic property `List<UserRepository>` must surface raw `List`: {refs:?}"
    );
}

#[test]
fn annotation_on_function_does_not_emit_property_reference() {
    // @-annotation on a function (e.g., @Composable fun Greet) must NOT
    // produce a property-level AnnotatedFieldType ref. Only property
    // annotations are in scope for this emitter.
    let src = b"\
@Composable\n\
fun Greet(name: String) { Text(\"Hi\") }\n";
    let refs = refs_in(src);
    let annotated: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert!(
        annotated.is_empty(),
        "@-annotation on function must NOT emit property AnnotatedFieldType ref: {annotated:?}"
    );
}

#[test]
fn empty_source_returns_no_references() {
    let refs = refs_in(b"");
    assert!(refs.is_empty());
}

#[test]
fn malformed_kotlin_source_does_not_panic_in_references_walker() {
    let garbage: &[u8] = b"@ }}}{ <<< abandon var !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_references(Lang::Kotlin, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_references panicked on garbage Kotlin input"
    );
}
