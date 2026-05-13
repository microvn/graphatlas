//! Tools S-002 cluster C — AS-005 TypeScript closure/arrow callees.
//!
//! Anonymous arrows invoked as callbacks (e.g. `.map(u => validateUser(u))`)
//! must record the OUTER function as the enclosing symbol so the indexer can
//! emit CALLS(outer → validateUser). Named arrows (`const foo = () => bar()`)
//! get their own name as enclosing.

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn anonymous_arrow_call_carries_outer_enclosing() {
    let src = r#"
function processUsers(users) {
    return users.map(u => validateUser(u));
}
function validateUser(u) { return u; }
"#;
    let calls = extract_calls(Lang::TypeScript, src.as_bytes()).unwrap();
    // Expected: one call-site for validateUser with enclosing=processUsers.
    let hits: Vec<_> = calls
        .iter()
        .filter(|c| c.callee_name == "validateUser")
        .collect();
    assert_eq!(hits.len(), 1, "{calls:?}");
    assert_eq!(
        hits[0].enclosing_symbol.as_deref(),
        Some("processUsers"),
        "{:?}",
        hits[0]
    );
}

#[test]
fn named_arrow_const_is_own_enclosing() {
    // `const foo = () => bar()` — arrow binds to declarator name `foo`.
    let src = r#"
const foo = () => bar();
function bar() {}
"#;
    let calls = extract_calls(Lang::TypeScript, src.as_bytes()).unwrap();
    let hits: Vec<_> = calls.iter().filter(|c| c.callee_name == "bar").collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].enclosing_symbol.as_deref(), Some("foo"));
}

#[test]
fn nested_arrow_still_attributes_to_outer() {
    // Arrow nested 2 levels deep inside processUsers — call still attributes
    // to processUsers (neither arrow has a name).
    let src = r#"
function processUsers(users) {
    return users.filter(u => u.active).map(u => validateUser(u));
}
function validateUser(u) {}
"#;
    let calls = extract_calls(Lang::TypeScript, src.as_bytes()).unwrap();
    let hits: Vec<_> = calls
        .iter()
        .filter(|c| c.callee_name == "validateUser")
        .collect();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].enclosing_symbol.as_deref(), Some("processUsers"));
}
