//! Foundation-C15 — extract_references: value-reference extraction tests.

use ga_core::Lang;
use ga_parser::references::{extract_references, RefKind};

#[test]
fn ts_object_pair_value_is_reference() {
    let src = br#"
function handleUsers() {}
function handlePosts() {}
function setup() {
  const routes = { '/api/users': handleUsers, '/api/posts': handlePosts };
  return routes;
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    let names: Vec<_> = refs.iter().map(|r| r.target_name.clone()).collect();
    assert!(names.contains(&"handleUsers".to_string()));
    assert!(names.contains(&"handlePosts".to_string()));
    let users = refs
        .iter()
        .find(|r| r.target_name == "handleUsers")
        .unwrap();
    assert_eq!(users.ref_kind, RefKind::MapValue);
    assert_eq!(users.enclosing_symbol.as_deref(), Some("setup"));
}

#[test]
fn ts_array_element_identifier_is_reference() {
    let src = br#"
function onStart() {}
function onDone() {}
function init() {
  const callbacks = [onStart, onDone];
  return callbacks;
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    let names: Vec<_> = refs.iter().map(|r| r.target_name.clone()).collect();
    assert!(names.contains(&"onStart".to_string()));
    assert!(names.contains(&"onDone".to_string()));
    assert_eq!(
        refs.iter()
            .find(|r| r.target_name == "onStart")
            .unwrap()
            .ref_kind,
        RefKind::ArrayElem
    );
}

#[test]
fn ts_shorthand_property_is_reference() {
    let src = br#"
function handleClick() {}
function wire() {
  const handlers = { handleClick };
  return handlers;
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    let h = refs.iter().find(|r| r.target_name == "handleClick");
    assert!(h.is_some(), "shorthand must emit a reference");
    assert_eq!(h.unwrap().ref_kind, RefKind::Shorthand);
}

#[test]
fn python_dict_pair_value_is_reference() {
    let src = br#"
def handle_users(): pass
def handle_posts(): pass

def setup():
    routes = {'/api/users': handle_users, '/api/posts': handle_posts}
    return routes
"#;
    let refs = extract_references(Lang::Python, src).unwrap();
    let names: Vec<_> = refs.iter().map(|r| r.target_name.clone()).collect();
    assert!(names.contains(&"handle_users".to_string()));
    assert!(names.contains(&"handle_posts".to_string()));
}

#[test]
fn stopword_keywords_not_emitted() {
    let src = br#"
function setup() {
  const m = { a: null, b: true, c: false, d: undefined };
  const l = [null, true, false];
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    for r in &refs {
        assert!(
            !matches!(
                r.target_name.as_str(),
                "null" | "true" | "false" | "undefined"
            ),
            "stopword kept: {}",
            r.target_name
        );
    }
}

#[test]
fn all_caps_constants_not_emitted() {
    let src = br#"
function setup() {
  const m = { a: CONST_ONE, b: CONST_TWO };
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    assert!(
        refs.iter().all(|r| r.target_name != "CONST_ONE"),
        "ALL_CAPS leaked through"
    );
}

#[test]
fn single_char_names_not_emitted() {
    let src = br#"
function setup() {
  const m = { x: a, y: b };
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    assert!(refs.iter().all(|r| r.target_name.len() > 1));
}

// infra:S-001 (v1.1-M0) — Go/Rust REFERENCES extraction.
//
// Replaces the pre-S-001 `go_rust_return_empty_deferred_per_foundation_c15`
// regression guard. Deferred behavior lifted for Go struct-field fn
// assignment (RefKind::StructFieldFn) and Rust fn pointer argument
// (RefKind::FnPointerArg). Other Go/Rust patterns (map value, slice elem,
// composite literal positional) intentionally still deferred — see Not in
// Scope in graphatlas-v1.1-infra.md.

/// AS-001 happy path — Go struct-field function assignment.
#[test]
fn go_struct_field_fn_assignment_emits_reference() {
    let src = br#"package main

type Handler struct {
	OnClick func()
}

func handleClick() {}

func register() {
	h := &Handler{OnClick: handleClick}
	_ = h
}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    let hit = refs
        .iter()
        .find(|r| r.target_name == "handleClick")
        .expect("AS-001: Go struct-field fn assignment must emit reference");
    assert_eq!(hit.ref_kind, RefKind::StructFieldFn);
    assert_eq!(hit.enclosing_symbol.as_deref(), Some("register"));
}

/// AS-002 happy path — Rust fn pointer passed as argument.
#[test]
fn rust_fn_pointer_argument_emits_reference() {
    let src = br#"
fn on_click() {}

fn register(cb: fn()) {
    cb();
}

fn main() {
    register(on_click);
}
"#;
    let refs = extract_references(Lang::Rust, src).unwrap();
    let hit = refs
        .iter()
        .find(|r| r.target_name == "on_click")
        .expect("AS-002: Rust fn pointer arg must emit reference");
    assert_eq!(hit.ref_kind, RefKind::FnPointerArg);
    assert_eq!(hit.enclosing_symbol.as_deref(), Some("main"));
}

/// AS-003 — Go unexported same-package: emission MUST happen (resolution
/// layer decides cross-package; parser just surfaces the structural site).
#[test]
fn go_unexported_same_package_emits() {
    let src = br#"package internal

type Bus struct {
	OnEvent func()
}

func helper() {} // unexported (lowercase)

func wire() {
	b := &Bus{OnEvent: helper}
	_ = b
}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    let hit = refs
        .iter()
        .find(|r| r.target_name == "helper")
        .expect("AS-003: same-package unexported Go reference must emit");
    assert_eq!(hit.ref_kind, RefKind::StructFieldFn);
}

/// AS-004 — malformed Go source returns ParseError (not panic, not empty).
#[test]
fn go_malformed_source_returns_parse_error_not_panic() {
    // Valid prefix then unclosed brace at EOF — tree-sitter produces an
    // error node but `parse()` still returns a tree; we want the extractor
    // to return Ok with a best-effort record set, NOT panic.
    let src = b"package main\nfunc broken() {\n"; // missing closing `}`
    let result = extract_references(Lang::Go, src);
    // Per R12 parse tolerance — return Ok with partial (possibly empty)
    // AST, not Err; indexer-level `parse_errors` flagging handles file
    // quality at build time.
    assert!(
        result.is_ok(),
        "AS-004: parser must not panic on malformed Go; got {:?}",
        result
    );
}

/// AS-004 — same contract for Rust.
#[test]
fn rust_malformed_source_returns_ok_not_panic() {
    let src = b"fn broken( {\n"; // invalid syntax
    let result = extract_references(Lang::Rust, src);
    assert!(
        result.is_ok(),
        "AS-004: parser must not panic on malformed Rust; got {:?}",
        result
    );
}

/// Stopword filter applies to Go/Rust too — `nil` should not leak.
#[test]
fn go_nil_stopword_filtered() {
    let src = br#"package main

type Handler struct { Cb func() }

func setup() {
	h := &Handler{Cb: nil}
	_ = h
}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    assert!(
        refs.iter().all(|r| r.target_name != "nil"),
        "Go stopword 'nil' leaked through"
    );
}

#[test]
fn empty_source_returns_empty() {
    assert!(extract_references(Lang::TypeScript, b"")
        .unwrap()
        .is_empty());
    assert!(extract_references(Lang::Python, b"").unwrap().is_empty());
    assert!(extract_references(Lang::Go, b"").unwrap().is_empty());
    assert!(extract_references(Lang::Rust, b"").unwrap().is_empty());
}
