//! v1.1-M4 S-002b — Kotlin CALLS extraction (AS-006) + parse tolerance.
//!
//! AS-006 (Kotlin CALLS — happy path):
//!   Given: Kotlin source with call_expression(s) inside a function body.
//!   When: extract_calls(Lang::Kotlin, source).
//!   Then: ParsedCall { enclosing_symbol: Some(<containing function>),
//!                      callee_name: <method name only — strips receiver>,
//!                      call_site_line: <1-based line> }.
//!
//! AS-005-equivalent (Parse failure tolerance — Lang-C1 atomic gate):
//!   Given: malformed Kotlin source.
//!   When: extract_calls is invoked.
//!   Then: returns Ok with partial calls from valid portions; never panics
//!         (R12 contract).

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn bare_call_inside_function_emits_parsed_call_with_enclosing() {
    // AS-006 canonical: `class Foo { fun bar() = baz() }` — bare `baz()` is
    // a `call_expression` whose first child is `identifier`. Enclosing must
    // be the containing function `bar`.
    let src = b"class Foo { fun bar() = baz() }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    let baz = calls
        .iter()
        .find(|c| c.callee_name == "baz")
        .unwrap_or_else(|| panic!("baz not found in calls: {calls:?}"));
    assert_eq!(
        baz.enclosing_symbol.as_deref(),
        Some("bar"),
        "enclosing must be the containing function bar"
    );
}

#[test]
fn qualified_call_uses_trailing_method_name() {
    // `obj.findById(id)` → callee_name = "findById" (NOT "obj"). The first
    // child is `navigation_expression` (receiver.method); the trailing
    // identifier is the method name.
    let src = b"\
class Repo { fun get() { obj.findById(1) } }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "findById"),
        "qualified `obj.findById(1)` must surface findById (NOT obj): {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.callee_name == "obj"),
        "callee_name must never be the receiver `obj`: {calls:?}"
    );
}

#[test]
fn static_qualified_call_uses_trailing_method_name() {
    // `Collections.emptyList()` → callee_name = "emptyList".
    let src = b"\
class Util { fun empty() = Collections.emptyList() }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "emptyList"),
        "static `Collections.emptyList()` must surface emptyList: {calls:?}"
    );
}

#[test]
fn constructor_call_emits_class_name_as_callee() {
    // `User()` (no `new` keyword in Kotlin) is a `call_expression` whose
    // first child is an `identifier`. The callee_name is the class name —
    // resolution to a constructor vs a function call is the indexer's job
    // (callee_name is just the identifier).
    let src = b"\
class Factory { fun make() = User() }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "User"),
        "constructor-style call `User()` must surface User as callee: {calls:?}"
    );
}

#[test]
fn chained_calls_emit_separate_calls() {
    // `b.append("a").append("b")` → both `append` calls surface.
    let src = b"\
class Chain { fun run(b: StringBuilder) { b.append(\"a\").append(\"b\") } }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    let appends = calls.iter().filter(|c| c.callee_name == "append").count();
    assert!(
        appends >= 2,
        "chained .append().append() must emit 2 calls; got {appends}: {calls:?}"
    );
}

#[test]
fn empty_kotlin_source_returns_ok_empty_calls() {
    let calls = extract_calls(Lang::Kotlin, b"").expect("extract_calls Ok on empty");
    assert!(calls.is_empty());
}

#[test]
fn declaration_only_kotlin_source_returns_ok_empty() {
    // Pure declaration — no call_expressions.
    let src = b"class Empty { val x: Int = 0 }\n";
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    assert!(
        calls.is_empty(),
        "declaration-only source must produce no calls: {calls:?}"
    );
}

#[test]
fn malformed_kotlin_source_returns_ok_with_partial_parse() {
    // Lang-C1 parse tolerance (AS-005-equiv): tree-sitter is permissive —
    // it produces a partial AST for malformed input. The valid `helper()`
    // call should still surface even though the surrounding class body is
    // broken.
    let src = b"\
class Broken { fun main() { helper() }\n\
fun unfinished( {  // <- missing closing paren / brace\n";
    let result = extract_calls(Lang::Kotlin, src);
    assert!(
        result.is_ok(),
        "Lang-C1 parse tolerance: malformed Kotlin must return Ok with partial parse, got: {:?}",
        result.err()
    );
    let calls = result.unwrap();
    assert!(
        calls.iter().any(|c| c.callee_name == "helper"),
        "Lang-C1: helper() in the valid portion must still surface: {calls:?}"
    );
}

#[test]
fn malformed_kotlin_source_does_not_panic() {
    // Lang-C1 defense-in-depth: even a totally broken byte stream must not
    // panic. The result may be Ok(empty) or Err — both acceptable; the
    // critical invariant is "caller can recover".
    let garbage: &[u8] =
        b"class }}}{ <<< abandon ship !!! ;:.,?/\\ \x01\x02\x03\xff\xfe fun fun fun";
    let result = std::panic::catch_unwind(|| extract_calls(Lang::Kotlin, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_calls panicked on garbage Kotlin input"
    );
}

#[test]
fn call_with_string_arg_does_not_lose_call() {
    // Special characters / unicode in arguments: callee resolution must
    // not be broken by the value_arguments tokens.
    let src = "class T { fun run() { greet(\"héllo wörld 🌍\") } }\n".as_bytes();
    let calls = extract_calls(Lang::Kotlin, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "greet"),
        "unicode/special-char string args must not corrupt callee extraction: {calls:?}"
    );
}
