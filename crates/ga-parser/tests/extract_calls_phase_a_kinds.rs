//! v1.1-M4 (S-005a D4) — regression net for `extract_call_name` per-kind
//! branches BEFORE migration to fn-pointer table.
//!
//! These behaviors were previously hardcoded as `if kind == "decorator"`,
//! `if kind == "new_expression"`, `if kind == "jsx_*"` inside the engine
//! `calls.rs::extract_call_name`. Pre-D4 engine passes these; post-D4
//! engine reads via `spec.callee_extractors()` and MUST also pass.

use ga_core::Lang;
use ga_parser::{extract_calls, ParsedCall};

fn calls_in(lang: Lang, src: &str) -> Vec<ParsedCall> {
    extract_calls(lang, src.as_bytes()).expect("extract_calls failed")
}

fn callee_names(calls: &[ParsedCall]) -> Vec<String> {
    calls.iter().map(|c| c.callee_name.clone()).collect()
}

// ---------------------------------------------------------------------------
// Python `decorator`
// ---------------------------------------------------------------------------

#[test]
fn python_decorator_simple_identifier_recorded() {
    // `@cache` — bare identifier decorator.
    let src = "\
@cache\n\
def fetch():\n    pass\n";
    let names = callee_names(&calls_in(Lang::Python, src));
    assert!(
        names.contains(&"cache".to_string()),
        "expected 'cache' decorator callee; got {names:?}"
    );
}

#[test]
fn python_decorator_attribute_returns_trailing_segment() {
    // `@app.route(...)` — decorator wraps a `call` whose function is
    // `attribute` (`app.route`). Engine returns the trailing identifier.
    let src = "\
@app.route('/users')\n\
def index():\n    pass\n";
    let names = callee_names(&calls_in(Lang::Python, src));
    assert!(
        names.contains(&"route".to_string()),
        "expected 'route' from `@app.route()`; got {names:?}"
    );
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript `new_expression`
// ---------------------------------------------------------------------------

#[test]
fn typescript_new_expression_simple_identifier() {
    let src = "\
class Foo {}\n\
function build() { const x = new Foo(); return x; }\n";
    let names = callee_names(&calls_in(Lang::TypeScript, src));
    assert!(
        names.contains(&"Foo".to_string()),
        "expected 'Foo' from `new Foo()`; got {names:?}"
    );
}

#[test]
fn javascript_new_expression_member_returns_property() {
    let src = "\
import pkg from 'm';\n\
function build() { const x = new pkg.Bar(); return x; }\n";
    let names = callee_names(&calls_in(Lang::JavaScript, src));
    assert!(
        names.contains(&"Bar".to_string()),
        "expected 'Bar' from `new pkg.Bar()`; got {names:?}"
    );
}

// ---------------------------------------------------------------------------
// JavaScript JSX uppercase = component reference
// ---------------------------------------------------------------------------

#[test]
fn javascript_jsx_uppercase_recorded_as_call() {
    let src = "\
function App() {\n\
  return <Greeting name=\"world\" />;\n\
}\n";
    let names = callee_names(&calls_in(Lang::JavaScript, src));
    assert!(
        names.contains(&"Greeting".to_string()),
        "JSX uppercase component must be recorded as call; got {names:?}"
    );
}

#[test]
fn javascript_jsx_lowercase_not_recorded() {
    // Lowercase JSX = HTML element, NOT a function/component reference.
    let src = "\
function App() {\n\
  return <div className=\"hero\" />;\n\
}\n";
    let names = callee_names(&calls_in(Lang::JavaScript, src));
    assert!(
        !names.contains(&"div".to_string()),
        "lowercase JSX `div` must NOT be recorded as call; got {names:?}"
    );
}

#[test]
fn javascript_jsx_opening_uppercase_recorded() {
    // jsx_opening_element (paired tag, not self-closing).
    let src = "\
function App() {\n\
  return <Container>hello</Container>;\n\
}\n";
    let names = callee_names(&calls_in(Lang::JavaScript, src));
    assert!(
        names.contains(&"Container".to_string()),
        "JSX opening tag (uppercase) must be recorded; got {names:?}"
    );
}
