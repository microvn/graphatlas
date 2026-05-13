//! v1.1-M4 S-003c — C# attributed-field/property REFERENCES (AS-004-equiv).
//!
//! Lang-C1 atomic UC gate. Lang-C7 PARTIAL for C#: ASP.NET Core mostly
//! uses constructor-injection; Blazor/Razor pages use `[Inject]` field
//! attributes. The mechanism (`RefKind::AnnotatedFieldType`) generalizes
//! from Java's `@Autowired` per the original Lang-C7 design.
//!
//! Tree-sitter shape:
//!   field_declaration
//!     ├── attribute_list { attribute: identifier "Inject" }
//!     ├── (modifier "private")
//!     └── variable_declaration
//!           ├── identifier "IUserRepository" (the type)
//!           └── variable_declarator → identifier "_repo"
//!
//!   property_declaration
//!     ├── attribute_list { attribute: identifier "Inject" }
//!     ├── (modifier "public")
//!     ├── identifier "IRepo"   (the type — direct child)
//!     ├── identifier "Repo"    (the property name)
//!     └── accessor_list

use ga_core::Lang;
use ga_parser::extract_references;
use ga_parser::references::RefKind;

fn refs_in(src: &[u8]) -> Vec<ga_parser::references::ParsedReference> {
    extract_references(Lang::CSharp, src).expect("extract_references Ok")
}

#[test]
fn attributed_field_emits_reference_to_type() {
    let src = b"class Service {\n    [Inject]\n    private IUserRepository _repo;\n}\n";
    let refs = refs_in(src);
    let r = refs
        .iter()
        .find(|r| r.target_name == "IUserRepository")
        .unwrap_or_else(|| panic!("no reference to IUserRepository: {refs:?}"));
    assert!(
        matches!(r.ref_kind, RefKind::AnnotatedFieldType),
        "Lang-C7 PARTIAL: attributed field ref must use AnnotatedFieldType, got {:?}",
        r.ref_kind
    );
}

#[test]
fn attributed_property_emits_reference_to_type() {
    let src = b"class Service {\n    [Inject]\n    public IRepo Repo { get; set; }\n}\n";
    let refs = refs_in(src);
    assert!(
        refs.iter()
            .any(|r| r.target_name == "IRepo" && matches!(r.ref_kind, RefKind::AnnotatedFieldType)),
        "[Inject] property must emit AnnotatedFieldType ref to type: {refs:?}"
    );
}

#[test]
fn field_without_attribute_emits_no_reference() {
    let src = b"class Service {\n    private IRepo _repo;\n}\n";
    let refs = refs_in(src);
    let annotated: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert!(
        annotated.is_empty(),
        "non-attributed field must NOT emit AnnotatedFieldType ref: {annotated:?}"
    );
}

#[test]
fn multiple_attributes_on_same_field_emit_single_ref() {
    let src = b"class Service {\n    [Inject] [Required] private IRepo _repo;\n}\n";
    let refs = refs_in(src);
    let urefs: Vec<_> = refs
        .iter()
        .filter(|r| r.target_name == "IRepo")
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert_eq!(
        urefs.len(),
        1,
        "multi-attribute must dedup to one ref per field: {urefs:?}"
    );
}

#[test]
fn qualified_field_type_strips_to_trailing_identifier() {
    let src = b"class Service {\n    [Inject] private System.Collections.IList _items;\n}\n";
    let refs = refs_in(src);
    assert!(
        refs.iter()
            .any(|r| r.target_name == "IList" && matches!(r.ref_kind, RefKind::AnnotatedFieldType)),
        "qualified type must surface trailing `IList`: {refs:?}"
    );
    assert!(
        !refs
            .iter()
            .any(|r| r.target_name == "System.Collections.IList"),
        "must NOT leak FQN into target_name: {refs:?}"
    );
}

#[test]
fn attribute_on_method_does_not_emit_field_reference() {
    let src = b"class Service {\n    [TestMethod]\n    public void Run() { }\n}\n";
    let refs = refs_in(src);
    let annotated: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r.ref_kind, RefKind::AnnotatedFieldType))
        .collect();
    assert!(
        annotated.is_empty(),
        "[Attribute] on method must NOT emit field-level AnnotatedFieldType: {annotated:?}"
    );
}

#[test]
fn empty_source_returns_no_references() {
    let refs = refs_in(b"");
    assert!(refs.is_empty());
}

#[test]
fn malformed_csharp_source_does_not_panic_in_references_walker() {
    let garbage: &[u8] = b"[ }}}{ <<< abandon class !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_references(Lang::CSharp, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_references panicked on garbage C# input"
    );
}
