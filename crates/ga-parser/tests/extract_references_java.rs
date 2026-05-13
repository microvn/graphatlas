//! v1.1-M4 S-001c — Java REFERENCES via annotated fields (AS-004).
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-004):
//!   Given: `@Service class UserService { @Autowired private UserRepository repo; }`
//!   When: indexed
//!   Then: `ga_impact {symbol: "UserService"}` surfaces files with
//!         `@Autowired UserService` via REFERENCES edge (annotation
//!         treated as reference).
//!
//! Pinned shape at the parser layer: an annotation on a `field_declaration`
//! emits a `ParsedReference { target_name: <field type>, ref_kind:
//! RefKind::AnnotatedFieldType }`. Annotations on methods/classes do NOT
//! emit field references (those go through `extract_attributes` per
//! S-005a D5 — out of scope for AS-004).

use ga_core::Lang;
use ga_parser::{extract_references, RefKind};

#[test]
fn autowired_field_emits_reference_to_field_type() {
    // AS-004 canonical example.
    let src = b"\
public class UserService {\n\
    @Autowired\n\
    private UserRepository repo;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    let r = refs
        .iter()
        .find(|r| r.target_name == "UserRepository")
        .unwrap_or_else(|| panic!("UserRepository ref must be emitted: {refs:?}"));
    assert_eq!(
        r.ref_kind,
        RefKind::AnnotatedFieldType,
        "ref_kind must be AnnotatedFieldType for @Autowired field"
    );
}

#[test]
fn autowired_field_reference_strips_module_prefix() {
    let src = b"\
public class UserService {\n\
    @Autowired\n\
    private com.example.UserRepository repo;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter().any(|r| r.target_name == "UserRepository"),
        "qualified field type must surface trailing class name: {refs:?}"
    );
}

#[test]
fn autowired_generic_field_emits_raw_type_reference() {
    // `@Autowired private List<UserRepository> repos;` — emit ref to the
    // raw type `List`. (Indexer / impact layer can decide later whether
    // to widen to type parameters.)
    let src = b"\
import java.util.List;\n\
public class UserService {\n\
    @Autowired\n\
    private List<UserRepository> repos;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter().any(|r| r.target_name == "List"),
        "generic field type must surface raw type `List`: {refs:?}"
    );
}

#[test]
fn annotation_on_method_does_not_emit_field_reference() {
    // `@Override` on a method must NOT emit a field reference — it's
    // a method-level annotation. AS-004 scope is field-level only.
    let src = b"\
public class Sub extends Base {\n\
    @Override\n\
    public void run() { super.run(); }\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter()
            .all(|r| r.ref_kind != RefKind::AnnotatedFieldType),
        "method-level @Override must NOT emit AnnotatedFieldType: {refs:?}"
    );
}

#[test]
fn annotation_on_class_does_not_emit_field_reference() {
    // `@Service` / `@Component` on a class is a class-level marker
    // (S-005a D5 routes it via extract_attributes). It must NOT emit
    // an AnnotatedFieldType reference.
    let src = b"\
@Service\n\
public class UserService {\n\
    public User get() { return null; }\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter()
            .all(|r| r.ref_kind != RefKind::AnnotatedFieldType),
        "class-level @Service must NOT emit AnnotatedFieldType: {refs:?}"
    );
}

#[test]
fn multiple_annotations_on_same_field_emit_single_reference() {
    // `@Autowired @Lazy private UserRepository repo;` — the field has
    // two annotations but represents ONE dependency. Emit one ref.
    let src = b"\
public class UserService {\n\
    @Autowired\n\
    @Lazy\n\
    private UserRepository repo;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    let count = refs
        .iter()
        .filter(|r| r.target_name == "UserRepository" && r.ref_kind == RefKind::AnnotatedFieldType)
        .count();
    assert_eq!(
        count, 1,
        "multi-annotated field must emit ONE ref to the type, not one per annotation: {refs:?}"
    );
}

#[test]
fn annotation_with_value_also_triggers_reference() {
    // `@Inject(name = "primary")` is `annotation` (not `marker_annotation`)
    // in tree-sitter-java grammar — the emitter must handle both kinds.
    let src = b"\
public class UserService {\n\
    @Inject(name = \"primary\")\n\
    private UserRepository repo;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter().any(|r| {
            r.target_name == "UserRepository" && r.ref_kind == RefKind::AnnotatedFieldType
        }),
        "valued @Inject(...) on field must still emit AnnotatedFieldType ref: {refs:?}"
    );
}

#[test]
fn no_annotation_no_reference() {
    // Plain field without annotation — no AnnotatedFieldType ref.
    let src = b"\
public class UserService {\n\
    private UserRepository repo;\n\
}\n";
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter()
            .all(|r| r.ref_kind != RefKind::AnnotatedFieldType),
        "plain field without annotation must NOT emit AnnotatedFieldType: {refs:?}"
    );
}

#[test]
fn empty_java_source_returns_ok_empty_refs() {
    let refs = extract_references(Lang::Java, b"").expect("Ok");
    assert!(refs.is_empty());
}

#[test]
fn malformed_java_source_does_not_panic_in_references_walker() {
    let garbage: &[u8] = b"public class }}}{ @Autowired @ private \x00\x01\xff void void void";
    let result = std::panic::catch_unwind(|| extract_references(Lang::Java, garbage));
    assert!(
        result.is_ok(),
        "extract_references panicked on garbage Java input"
    );
}

// ─── Edge-Case Compliance (skill Phase 1, mandatory rows) ───
//
// Boundary (large data) + Special characters rows. The remaining rows
// (Null/Invalid/Race/LargeData) are N/A at the type / runtime layer.

#[test]
fn many_annotated_fields_emit_one_ref_each() {
    // Boundary / large-data: 50 @Autowired fields with distinct types
    // must each emit exactly one AnnotatedFieldType ref. No truncation,
    // no emit-twice, no overflow.
    let mut src = String::from("public class Hub {\n");
    for i in 0..50 {
        src.push_str(&format!("    @Autowired private Type{i} f{i};\n"));
    }
    src.push_str("}\n");

    let refs = extract_references(Lang::Java, src.as_bytes()).expect("Ok");
    for i in 0..50 {
        let want = format!("Type{i}");
        let count = refs
            .iter()
            .filter(|r| r.target_name == want && r.ref_kind == RefKind::AnnotatedFieldType)
            .count();
        assert_eq!(
            count, 1,
            "expected exactly one AnnotatedFieldType ref for {want} (got {count})"
        );
    }
}

#[test]
fn unicode_in_source_does_not_corrupt_ref_extraction() {
    // Special chars: a Unicode comment alongside ASCII annotated fields
    // must not break the walker. Tree-sitter-java's Unicode-identifier
    // semantics are not pinned here (per AS-002 / AS-003 sibling tests).
    let src = "// 日本語コメント\npublic class S { @Autowired private UserRepository repo; }\n"
        .as_bytes();
    let refs = extract_references(Lang::Java, src).expect("Ok");
    assert!(
        refs.iter().any(|r| {
            r.target_name == "UserRepository" && r.ref_kind == RefKind::AnnotatedFieldType
        }),
        "ASCII annotated field must extract cleanly with Unicode comment in file: {refs:?}"
    );
}
