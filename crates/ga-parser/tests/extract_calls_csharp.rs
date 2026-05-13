//! v1.1-M4 S-003b — C# CALLS extraction (AS-009) + parse tolerance.
//!
//! AS-009 (C# CALLS — happy path):
//!   Given: `namespace App { class Service { public void Process() { helper.Run(); } } }`
//!   When: extract_calls(Lang::CSharp, source).
//!   Then: ParsedCall { enclosing_symbol: Some("Process"),
//!                      callee_name: "Run",
//!                      call_site_line: ... }.
//!
//! Plus AS-005-equivalent (Lang-C1 parse tolerance):
//!   Given: malformed C# source.
//!   When: extract_calls invoked.
//!   Then: returns Ok with partial calls; never panics.

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn invocation_inside_method_emits_parsed_call_with_enclosing() {
    // AS-009 canonical example from spec.
    let src = b"\
namespace App {\n\
    class Service {\n\
        public void Process() {\n\
            helper.Run();\n\
        }\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let run = calls
        .iter()
        .find(|c| c.callee_name == "Run")
        .unwrap_or_else(|| panic!("Run not found in calls: {calls:?}"));
    assert_eq!(
        run.enclosing_symbol.as_deref(),
        Some("Process"),
        "enclosing must be containing method `Process`"
    );
}

#[test]
fn qualified_invocation_uses_trailing_method_name() {
    // `obj.FindById(id)` → callee_name = "FindById" (NOT "obj").
    let src = b"\
class Repo { public void Get() { obj.FindById(1); } }\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "FindById"),
        "qualified `obj.FindById(1)` must surface FindById (NOT obj): {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.callee_name == "obj"),
        "callee_name must never be the receiver `obj`: {calls:?}"
    );
}

#[test]
fn fully_qualified_invocation_uses_trailing_method_name() {
    // `System.Console.WriteLine("hi")` → callee_name = "WriteLine".
    let src = b"\
class Util { void Log() { System.Console.WriteLine(\"hi\"); } }\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "WriteLine"),
        "fully-qualified `System.Console.WriteLine(...)` must surface WriteLine: {calls:?}"
    );
}

#[test]
fn object_creation_emits_class_name_as_callee() {
    // `new User(...)` → callee_name = "User".
    let src = b"\
class Factory { public User Make() { return new User(\"a\"); } }\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "User"),
        "constructor `new User(...)` must surface User as callee: {calls:?}"
    );
}

#[test]
fn chained_invocations_emit_separate_calls() {
    // `b.Append("a").Append("b")` → both Append calls.
    let src = b"\
class Chain { void Run(StringBuilder b) { b.Append(\"a\").Append(\"b\"); } }\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let appends = calls.iter().filter(|c| c.callee_name == "Append").count();
    assert!(
        appends >= 2,
        "chained .Append().Append() must emit 2 calls; got {appends}: {calls:?}"
    );
}

#[test]
fn empty_csharp_source_returns_ok_empty_calls() {
    let calls = extract_calls(Lang::CSharp, b"").expect("extract_calls Ok on empty");
    assert!(calls.is_empty());
}

#[test]
fn declaration_only_csharp_source_returns_ok_empty() {
    // Pure declaration — no calls.
    let src = b"namespace N { class C { public int X = 0; } }\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    assert!(
        calls.is_empty(),
        "declaration-only source must produce no calls: {calls:?}"
    );
}

#[test]
fn malformed_csharp_source_returns_ok_with_partial_parse() {
    // Lang-C1 parse tolerance: tree-sitter is permissive — partial AST.
    let src = b"\
class Broken { void Main() { Helper(); }\n\
void Unfinished( {  // missing brace\n";
    let result = extract_calls(Lang::CSharp, src);
    assert!(
        result.is_ok(),
        "Lang-C1 parse tolerance: malformed C# must return Ok, got: {:?}",
        result.err()
    );
    let calls = result.unwrap();
    assert!(
        calls.iter().any(|c| c.callee_name == "Helper"),
        "Lang-C1: Helper() in valid portion must still surface: {calls:?}"
    );
}

#[test]
fn malformed_csharp_source_does_not_panic() {
    let garbage: &[u8] = b"namespace }}}{ <<< abandon class !!! \x01\xff\xfe void void void";
    let result = std::panic::catch_unwind(|| extract_calls(Lang::CSharp, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_calls panicked on garbage C# input"
    );
}

#[test]
fn special_chars_in_string_arg_does_not_lose_call() {
    let src = "class T { void Run() { Greet(\"héllo wörld 🌍\"); } }\n".as_bytes();
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "Greet"),
        "unicode/special-char string args must not corrupt callee extraction: {calls:?}"
    );
}
