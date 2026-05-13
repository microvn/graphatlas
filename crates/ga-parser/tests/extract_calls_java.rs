//! v1.1-M4 S-001b — Java CALLS extraction (AS-001) + parse tolerance (AS-005).
//!
//! AS-001 (Java CALLS — happy path):
//!   Given: Java source with method_invocation(s) inside a method body.
//!   When: extract_calls(Lang::Java, source).
//!   Then: ParsedCall { enclosing_symbol: Some(<containing method>),
//!                      callee_name: <method name only>,
//!                      call_site_line: <1-based line> }.
//!
//! AS-005 (Parse failure tolerance):
//!   Given: malformed Java source.
//!   When: extract_calls is invoked.
//!   Then: returns Ok with partial calls from valid portions; never panics
//!         (R12 contract).

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn method_invocation_inside_method_emits_parsed_call_with_enclosing() {
    // AS-001 canonical example: UserService.getUser() calls
    // userRepository.findById(id). The walker must see method_declaration
    // as the enclosing symbol and emit a ParsedCall whose callee_name is
    // the trailing method (`findById`) — NOT the receiver (`userRepository`).
    let src = b"\
public class UserService {\n\
    UserRepository userRepository;\n\
    public User getUser(int id) {\n\
        return userRepository.findById(id);\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    let find_by_id = calls
        .iter()
        .find(|c| c.callee_name == "findById")
        .unwrap_or_else(|| panic!("findById not found in calls: {calls:?}"));
    assert_eq!(
        find_by_id.enclosing_symbol.as_deref(),
        Some("getUser"),
        "enclosing must be the containing method getUser"
    );
    assert_eq!(find_by_id.call_site_line, 4);
}

#[test]
fn object_creation_expression_emits_class_name_as_callee() {
    // `new User()` is a constructor call — surfaces as ParsedCall with
    // callee_name = class name. Mirrors the JS/TS new_expression pattern.
    let src = b"\
public class Factory {\n\
    public User make() { return new User(); }\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "User"),
        "object_creation_expression `new User()` must surface User as callee: {calls:?}"
    );
}

#[test]
fn object_creation_with_qualified_type_strips_module_prefix() {
    // `new com.example.Bar()` → callee_name = "Bar" (last segment).
    let src = b"\
public class Factory {\n\
    public Object make() { return new com.example.Bar(); }\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "Bar"),
        "qualified `new com.example.Bar()` must surface Bar (last segment): {calls:?}"
    );
}

#[test]
fn static_method_call_uses_trailing_method_name() {
    // `Collections.emptyList()` → callee_name = "emptyList" (NOT "Collections").
    let src = b"\
public class Util {\n\
    public java.util.List<String> empty() { return java.util.Collections.emptyList(); }\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "emptyList"),
        "static `Collections.emptyList()` must surface emptyList: {calls:?}"
    );
}

#[test]
fn chained_method_invocations_emit_separate_calls() {
    // `obj.first().second()` → both `first` and `second` are emitted.
    let src = b"\
public class Chain {\n\
    void run(StringBuilder b) { b.append(\"a\").append(\"b\"); }\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    let appends = calls.iter().filter(|c| c.callee_name == "append").count();
    assert!(
        appends >= 2,
        "chained .append().append() must emit 2 calls; got {appends}: {calls:?}"
    );
}

#[test]
fn unqualified_bare_method_call_emits_simple_name() {
    // Bare `helper()` (no receiver) inside a method emits callee_name=helper.
    let src = b"\
public class A {\n\
    void main() { helper(); }\n\
    void helper() {}\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    assert!(
        calls
            .iter()
            .any(|c| c.callee_name == "helper" && c.enclosing_symbol.as_deref() == Some("main")),
        "bare helper() inside main() must emit ParsedCall(callee=helper, enclosing=main): {calls:?}"
    );
}

#[test]
fn empty_java_source_returns_ok_empty_calls() {
    let calls = extract_calls(Lang::Java, b"").expect("extract_calls Ok on empty");
    assert!(calls.is_empty());
}

#[test]
fn no_calls_in_source_returns_ok_empty() {
    // Pure declaration, no invocations.
    let src = b"\
public class Empty {\n\
    private int x;\n\
}\n";
    let calls = extract_calls(Lang::Java, src).expect("extract_calls Ok");
    assert!(
        calls.is_empty(),
        "declaration-only source must produce no calls: {calls:?}"
    );
}

#[test]
fn malformed_java_source_returns_ok_with_partial_parse() {
    // AS-005: tree-sitter is permissive — it produces a partial AST for
    // malformed input. The valid `helper()` call should still surface even
    // though the surrounding class body is broken.
    let src = b"\
public class Broken {\n\
    void main() { helper(); }\n\
    void unfinished( {  // <- missing closing paren / brace\n\
}\n";
    let result = extract_calls(Lang::Java, src);
    assert!(
        result.is_ok(),
        "AS-005: malformed Java must return Ok with partial parse, got: {:?}",
        result.err()
    );
    let calls = result.unwrap();
    assert!(
        calls.iter().any(|c| c.callee_name == "helper"),
        "AS-005: helper() in the valid portion must still surface: {calls:?}"
    );
}

#[test]
fn malformed_java_source_does_not_panic() {
    // AS-005 defense-in-depth: even a totally broken byte stream must not
    // panic. The result may be Ok(empty) or Err — both acceptable; the
    // critical invariant is "caller can recover".
    let garbage: &[u8] =
        b"public class }}}{ <<< abandon ship !!! ;:.,?/\\ \x01\x02\x03\xff\xfe void void void";
    let result = std::panic::catch_unwind(|| extract_calls(Lang::Java, garbage));
    assert!(
        result.is_ok(),
        "AS-005: extract_calls panicked on garbage Java input"
    );
}
