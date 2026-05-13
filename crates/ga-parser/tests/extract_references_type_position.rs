//! Type-position references — Go, Rust, TS/JS.
//!
//! Driver: M3 dead_code FP audit found `ga_dead_code` over-flags
//! symbols as dead because the indexer's `extract_references` for
//! Go/Rust/TS doesn't emit edges for type-position uses.
//!
//! Concrete examples (gin):
//!   - `Accounts` defined in auth.go, used as `var x Accounts` in
//!     auth_test.go → no REFERENCES edge → engine flags dead.
//!   - `SliceValidationError` used as `&SliceValidationError{...}`
//!     in test files → same gap.
//!   - `yamlBinding` used as `yamlBinding{}` → same gap.
//!
//! These are real callers per raw text, but engine indexer's
//! references.rs only emits for keyed_element / fn_pointer_arg etc.
//!
//! Contract: when a type identifier (Go `type_identifier`, Rust
//! `type_identifier`, TS `type_identifier`) appears in a position that
//! references a previously-defined type, emit a `RefKind::TypePosition`
//! ParsedReference. The walker visits all type identifiers; we filter
//! out trivial cases (primitives, generics inside the def itself).

use ga_core::Lang;
use ga_parser::references::{extract_references, RefKind};

// ─────────── Go ───────────

#[test]
fn go_var_decl_type_position_is_reference() {
    // `var x Accounts` — the `Accounts` token is a type_identifier
    // referring to a previously defined struct.
    let src = br#"
package auth
type Accounts struct{}
func use_it() {
    var x Accounts
    _ = x
}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.target_name == "Accounts" && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `Accounts`; got: {:?}",
        refs
    );
}

#[test]
fn go_composite_literal_type_position_is_reference() {
    // `&SliceValidationError{}` — composite literal type.
    let src = br#"
package binding
type SliceValidationError struct{}
func make() interface{} {
    return &SliceValidationError{}
}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    assert!(
        refs.iter().any(|r| r.target_name == "SliceValidationError"
            && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `SliceValidationError`; got: {:?}",
        refs
    );
}

#[test]
fn go_function_param_type_position_is_reference() {
    // `func f(b yamlBinding)` — param type position.
    let src = br#"
package binding
type yamlBinding struct{}
func process(b yamlBinding) {}
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.target_name == "yamlBinding" && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `yamlBinding`; got: {:?}",
        refs
    );
}

// ─────────── Rust ───────────

#[test]
fn rust_let_binding_type_position_is_reference() {
    let src = br#"
struct Accounts;
fn use_it() {
    let x: Accounts;
}
"#;
    let refs = extract_references(Lang::Rust, src).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.target_name == "Accounts" && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `Accounts`; got: {:?}",
        refs
    );
}

#[test]
fn rust_function_return_type_position_is_reference() {
    let src = br#"
struct Router;
fn make() -> Router { Router }
"#;
    let refs = extract_references(Lang::Rust, src).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.target_name == "Router" && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `Router` from return type; got: {:?}",
        refs
    );
}

// ─────────── TypeScript ───────────

#[test]
fn ts_let_annotation_type_position_is_reference() {
    let src = br#"
class UserRepository {}
function setup() {
    let x: UserRepository;
}
"#;
    let refs = extract_references(Lang::TypeScript, src).unwrap();
    assert!(
        refs.iter()
            .any(|r| r.target_name == "UserRepository" && r.ref_kind == RefKind::TypePosition),
        "expected TypePosition ref to `UserRepository`; got: {:?}",
        refs
    );
}

// ─────────── Regression: don't spam refs ───────────

#[test]
fn primitive_types_are_not_emitted() {
    // Go primitives `int`, `string`, `bool` etc. — must NOT be emitted
    // as TypePosition refs (would explode the universe).
    let src = br#"
package x
func add(a int, b int) string { return "" }
"#;
    let refs = extract_references(Lang::Go, src).unwrap();
    let prim_refs: Vec<&str> = refs
        .iter()
        .filter(|r| r.ref_kind == RefKind::TypePosition)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        !prim_refs.contains(&"int"),
        "primitive `int` must not be a TypePosition ref"
    );
    assert!(
        !prim_refs.contains(&"string"),
        "primitive `string` must not be a TypePosition ref"
    );
}
