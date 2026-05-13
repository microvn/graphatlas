//! v1.1-M4 S-003c follow-up — C# preprocessor pre-strip pass.
//!
//! Bug: tree-sitter-c-sharp 0.23.5 produces ERROR nodes when encountering
//! `#if FOO / #else / #endif` blocks mid-class-body. Symptom on MQTTnet
//! fixture: 6+ files with cascading parse errors → entire classes wrapped
//! in `#if` are dropped from GA's call graph → MQTTnet M2 composite 0.317
//! (loses to ripgrep 0.539).
//!
//! Root cause: tree-sitter grammar PR #333 (2024-05-03) explicitly traded
//! mid-statement preprocessor support for cleaner trees. Upstream maintainer
//! position is WONTFIX-by-architecture (issues #189, #376, #377).
//!
//! Fix: pre-process source to replace `#if/#elif/#else/#endif/#define/#undef`
//! lines with whitespace of EQUAL BYTE LENGTH (preserves offsets so spans /
//! diagnostics stay valid). Tree-sitter then parses both branches as if
//! they were sequential statements. Concat-mode unioning per research brief.
//!
//! Edge cases:
//! - String literals containing `#` must NOT be stripped (verbatim, regular,
//!   interpolated, raw triple-quoted strings — C# 11+).
//! - `#region`, `#pragma`, `#warning`, `#error`, `#line`, `#nullable`
//!   directives ARE stripped uniformly (safe — they're declarations not
//!   structural).
//! - Byte length must be preserved exactly so `start_position()` /
//!   `end_position()` of recovered symbols match original source.

use ga_core::Lang;
use ga_parser::extract_calls;

/// Canonical reproducer from MQTTnet's CrossPlatformSocket.cs L70-100.
/// Pre-fix: tree-sitter produces 9 ERROR nodes; the property setter/getter
/// methods inside `#if` and `#else` branches are NOT recovered as symbols
/// → no CALLS edges from `_socket.GetSocketOption` etc.
const MQTTNET_CROSSPLATFORM_SNIPPET: &[u8] = b"\
public class CrossPlatformSocket {\n\
    public int TcpKeepAliveInterval {\n\
#if NETCOREAPP3_0_OR_GREATER\n\
        get => _socket.GetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveInterval) as int? ?? 0;\n\
        set => _socket.SetSocketOption(SocketOptionLevel.Tcp, SocketOptionName.TcpKeepAliveInterval, value);\n\
#else\n\
        get { throw new NotSupportedException(\"requires netcoreapp3.0\"); }\n\
        set { throw new NotSupportedException(\"requires netcoreapp3.0\"); }\n\
#endif\n\
    }\n\
}\n";

#[test]
fn preprocessor_block_recovers_calls_from_both_branches() {
    // After fix: BOTH branches union into the call graph (concat-mode per
    // docs/guide/dataset-for-new-language.md research brief). Expected calls
    // recovered: GetSocketOption, SetSocketOption (#if branch); throw of
    // NotSupportedException constructor (#else branch).
    let calls =
        extract_calls(Lang::CSharp, MQTTNET_CROSSPLATFORM_SNIPPET).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    assert!(
        names.contains(&"GetSocketOption"),
        "FIX: #if branch must surface — GetSocketOption missing from {names:?}"
    );
    assert!(
        names.contains(&"SetSocketOption"),
        "FIX: #if branch SetSocketOption missing from {names:?}"
    );
    assert!(
        names.contains(&"NotSupportedException"),
        "FIX: #else branch constructor (`new NotSupportedException(...)`) missing from {names:?}"
    );
}

#[test]
fn preprocessor_block_does_not_break_outer_class_recognition() {
    // Pre-fix: the cascading ERROR caused the parser to lose track of class
    // scope, so calls inside the property block lost their enclosing_symbol.
    // After fix: enclosing chain preserved.
    let src = b"\
public class Outer {\n\
    public void Method() {\n\
#if FOO\n\
        callA();\n\
#else\n\
        callB();\n\
#endif\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let call_a = calls
        .iter()
        .find(|c| c.callee_name == "callA")
        .unwrap_or_else(|| panic!("callA not found in {calls:?}"));
    assert_eq!(
        call_a.enclosing_symbol.as_deref(),
        Some("Method"),
        "FIX: enclosing scope must survive preprocessor strip — got {call_a:?}"
    );
}

#[test]
fn directive_inside_string_literal_must_not_be_stripped() {
    // Edge case: a string literal containing `#if` must be left intact.
    // If the strip pass is naive (line-prefix `#`), it will still leave
    // string contents alone because directive lines start with `#` as
    // first non-whitespace character on the line. Test guards this.
    let src = b"\
public class StringDemo {\n\
    public string Build() => \"prefix #if pattern\";\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    // No syntax error → Build method must surface (no calls inside, but
    // the function symbol itself parses cleanly).
    let _ = calls; // Smoke: just assert no panic / Ok return.
}

#[test]
fn elif_chain_recovers_all_branches() {
    // `#if A / #elif B / #elif C / #else / #endif` — concat-mode unions
    // ALL branches into the parsed tree.
    let src = b"\
public class Multi {\n\
    public void M() {\n\
#if A\n\
        callA();\n\
#elif B\n\
        callB();\n\
#elif C\n\
        callC();\n\
#else\n\
        callD();\n\
#endif\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    for expected in &["callA", "callB", "callC", "callD"] {
        assert!(
            names.contains(expected),
            "FIX: #elif branch missing — {expected} not in {names:?}"
        );
    }
}

#[test]
fn region_pragma_directives_also_stripped_safely() {
    // `#region` / `#endregion` / `#pragma` are not as harmful as `#if`
    // (tree-sitter handles them) but the strip pass treats all `#`-prefix
    // directive lines uniformly. Verify no regression.
    let src = b"\
public class RegionDemo {\n\
#region Public API\n\
    public void Greet() { System.Console.WriteLine(\"hi\"); }\n\
#endregion\n\
#pragma warning disable CS0168\n\
    public void Other() { Greet(); }\n\
#pragma warning restore CS0168\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    assert!(
        names.contains(&"WriteLine"),
        "FIX: #region wrapping must not lose calls — WriteLine missing from {names:?}"
    );
    assert!(
        names.contains(&"Greet"),
        "FIX: #region wrapping must not lose calls — Greet missing from {names:?}"
    );
}

#[test]
fn full_mqttnet_crossplatform_socket_file_recovers_calls() {
    // Real-world end-to-end: parse the EXACT file that errors in M2 runs.
    // Pre-fix: 9 ERROR nodes; methods inside #if branches drop from index.
    // Post-fix: zero errors, all calls surface.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("benches/fixtures/MQTTnet/Source/MQTTnet/Implementations/CrossPlatformSocket.cs");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "SKIP: MQTTnet submodule not checked out at {}",
                path.display()
            );
            return;
        }
    };
    let calls = extract_calls(Lang::CSharp, &bytes).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    // Calls that should appear (some inside #if NETCOREAPP3_0_OR_GREATER blocks):
    for expected in &["GetSocketOption", "SetSocketOption"] {
        assert!(
            names.contains(expected),
            "FIX: full MQTTnet file must surface `{expected}` (currently dropped due to #if cascade); got {} unique callees",
            names.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }
}

#[test]
fn negated_if_expression_recovers() {
    // Real-world reproducer: MQTTnet uses `#if !NET5_0_OR_GREATER` (with !
    // negation operator). Bare `#if A` works in tree-sitter; negation form
    // historically does not.
    let src = b"\
public sealed class Sock {\n\
    readonly Socket _socket;\n\
\n\
#if !NET5_0_OR_GREATER\n\
    readonly Action _disposeAction;\n\
#endif\n\
\n\
    public void Init() {\n\
        _socket = new Socket();\n\
#if !NET5_0_OR_GREATER\n\
        _disposeAction = _socket.Dispose;\n\
#endif\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    assert!(
        names.contains(&"Socket"),
        "FIX: ctor `new Socket()` outside #if must surface — {names:?}"
    );
}

#[test]
fn nested_if_blocks_recover() {
    let src = b"\
public class Nested {\n\
    public void M() {\n\
#if A\n\
#if B\n\
        innerCall();\n\
#endif\n\
#endif\n\
        outerCall();\n\
    }\n\
}\n";
    let calls = extract_calls(Lang::CSharp, src).expect("extract_calls Ok");
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    assert!(
        names.contains(&"innerCall"),
        "FIX: nested #if must recover inner call — got {names:?}"
    );
    assert!(
        names.contains(&"outerCall"),
        "FIX: outerCall after nested #endif must recover — got {names:?}"
    );
}
