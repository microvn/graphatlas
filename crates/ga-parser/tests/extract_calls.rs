//! Tools S-001 cluster B — extract call edges from tree-sitter.
//!
//! For each call-node, record `(enclosing_symbol, callee_name, line)`. The
//! indexer turns these into `CALLS` graph edges.

use ga_core::Lang;
use ga_parser::{extract_calls, ParsedCall};

fn calls_in(lang: Lang, src: &str) -> Vec<ParsedCall> {
    extract_calls(lang, src.as_bytes()).expect("extract_calls")
}

#[test]
fn python_direct_call_tracked() {
    // def foo() {} / def bar(): foo()
    let src = "def foo():\n    pass\n\ndef bar():\n    foo()\n";
    let calls = calls_in(Lang::Python, src);
    assert_eq!(calls.len(), 1);
    let c = &calls[0];
    assert_eq!(c.enclosing_symbol.as_deref(), Some("bar"));
    assert_eq!(c.callee_name, "foo");
    assert_eq!(c.call_site_line, 5);
}

#[test]
fn python_method_call_via_self() {
    // class C: def greet(self): self.helper() / def helper(self): pass
    let src = "class C:\n    def greet(self):\n        self.helper()\n\n    def helper(self):\n        pass\n";
    let calls = calls_in(Lang::Python, src);
    assert_eq!(calls.len(), 1);
    let c = &calls[0];
    assert_eq!(c.enclosing_symbol.as_deref(), Some("greet"));
    assert_eq!(c.callee_name, "helper");
}

#[test]
fn python_top_level_call_has_no_enclosing() {
    // Module-level function call → no enclosing symbol.
    let src = "print('hi')\n";
    let calls = calls_in(Lang::Python, src);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].enclosing_symbol, None);
    assert_eq!(calls[0].callee_name, "print");
}

#[test]
fn python_multiple_calls_in_one_fn_all_recorded() {
    let src = "def orchestrate():\n    step_a()\n    step_b()\n    step_c()\n";
    let calls = calls_in(Lang::Python, src);
    assert_eq!(calls.len(), 3);
    let names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    for expected in ["step_a", "step_b", "step_c"] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
    assert!(calls
        .iter()
        .all(|c| c.enclosing_symbol.as_deref() == Some("orchestrate")));
}

#[test]
fn rust_function_call_tracked() {
    let src = "fn foo() {}\nfn bar() { foo(); }\n";
    let calls = calls_in(Lang::Rust, src);
    // Expect 1 CALLS edge: bar → foo.
    let named: Vec<_> = calls.iter().filter(|c| c.callee_name == "foo").collect();
    assert_eq!(named.len(), 1, "all calls: {calls:?}");
    assert_eq!(named[0].enclosing_symbol.as_deref(), Some("bar"));
}

#[test]
fn rust_method_call_via_dot() {
    // fn main() { let s = String::new(); s.push_str("x"); }
    let src = "fn main() { let s = String::new(); s.push_str(\"x\"); }\n";
    let calls = calls_in(Lang::Rust, src);
    let callee_names: Vec<&str> = calls.iter().map(|c| c.callee_name.as_str()).collect();
    // We expect to see "push_str" (via member_expression) and "new" (via scoped).
    assert!(callee_names.contains(&"push_str"), "{callee_names:?}");
    assert!(callee_names.contains(&"new"), "{callee_names:?}");
}

#[test]
fn typescript_member_call_yields_method_name() {
    let src = "function run(s: any) { s.process(); }\n";
    let calls = calls_in(Lang::TypeScript, src);
    assert!(calls.iter().any(|c| c.callee_name == "process"));
}

#[test]
fn go_function_call_tracked() {
    let src = "package main\n\nfunc main() {\n  hello()\n}\n\nfunc hello() {}\n";
    let calls = calls_in(Lang::Go, src);
    let into_main: Vec<_> = calls
        .iter()
        .filter(|c| c.enclosing_symbol.as_deref() == Some("main"))
        .collect();
    assert_eq!(into_main.len(), 1);
    assert_eq!(into_main[0].callee_name, "hello");
}

#[test]
fn empty_source_yields_no_calls() {
    for lang in [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
    ] {
        let calls = calls_in(lang, "");
        assert!(calls.is_empty(), "{lang:?} empty source: {calls:?}");
    }
}

#[test]
fn call_site_line_is_1_based() {
    let src = "\n\ndef a(): pass\n\ndef b():\n    a()\n";
    let calls = calls_in(Lang::Python, src);
    let c = calls.iter().find(|c| c.callee_name == "a").unwrap();
    assert_eq!(c.call_site_line, 6);
}

#[test]
fn rust_macro_invocation_recorded() {
    // println! should appear as a call of "println" (trailing ! stripped).
    let src = "fn main() { println!(\"hi\"); }\n";
    let calls = calls_in(Lang::Rust, src);
    assert!(
        calls.iter().any(|c| c.callee_name == "println"),
        "{calls:?}"
    );
}
