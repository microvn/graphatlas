//! Audit: Python decorator + register-arg call/reference extraction.
//!
//! Goal: verify whether ga-parser already emits enough edges to obviate
//! the dead_code.rs hardcoded lists DJANGO_VIEW_DECORATORS + the
//! `*/checks.py + check_*` rule.
//!
//! Per `/audit doc-as-truth 2026-05-01`:
//!   - DJANGO_VIEW_DECORATORS should be redundant if `@<name>` emits a
//!     CALLS edge naming `<name>` from the enclosing scope.
//!   - `*/checks.py + check_*` rule should be redundant if
//!     `register(check_foo)` emits a REFERENCES edge to `check_foo`
//!     (FnPointerArg-style) from the enclosing scope.

use ga_core::Lang;
use ga_parser::{extract_calls, extract_references, ParsedCall, ParsedReference};

fn calls(src: &str) -> Vec<ParsedCall> {
    extract_calls(Lang::Python, src.as_bytes()).expect("extract_calls")
}

fn refs(src: &str) -> Vec<ParsedReference> {
    extract_references(Lang::Python, src.as_bytes()).expect("extract_references")
}

// ─────────────────────────────────────────────────────────────────────────
// Q1: Does `@foo def view(): ...` emit a CALLS edge to `foo`?
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn audit_decorator_simple_emits_calls_edge() {
    // `@staff_member_required` decorating a top-level function.
    let src = "\
@staff_member_required
def admin_view():
    pass
";
    let cs = calls(src);
    let names: Vec<&str> = cs.iter().map(|c| c.callee_name.as_str()).collect();
    println!("[Q1] decorator-simple calls: {names:?}");
    assert!(
        names.contains(&"staff_member_required"),
        "decorator name should appear as callee — got {names:?}"
    );
}

#[test]
fn audit_decorator_with_args_emits_calls_edge() {
    // `@app.route("/")` — decorator wraps a call.
    let src = "\
@app.route(\"/\")
def index():
    pass
";
    let cs = calls(src);
    let names: Vec<&str> = cs.iter().map(|c| c.callee_name.as_str()).collect();
    println!("[Q1b] decorator-with-args calls: {names:?}");
    assert!(
        names.contains(&"route"),
        "decorator method name should appear — got {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Q2: Does `register(check_foo)` emit a REFERENCES edge to `check_foo`?
// (i.e. `check_foo` passed as an argument to a registration function)
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn audit_function_passed_as_arg_emits_reference() {
    // Module-level `register(check_db_setup)` — the function `check_db_setup`
    // is referenced by-value, not called.
    let src = "\
def check_db_setup():
    return 'ok'

register(check_db_setup)
";
    let rs = refs(src);
    let names: Vec<&str> = rs.iter().map(|r| r.target_name.as_str()).collect();
    println!("[Q2] register(check_X) refs: {names:?}");

    // The CALLS extractor would record `register` as a callee. We're asking:
    // is `check_db_setup` (the argument) recorded as a *reference* somewhere?
    let cs = calls(src);
    let call_names: Vec<&str> = cs.iter().map(|c| c.callee_name.as_str()).collect();
    println!("[Q2] register(check_X) calls: {call_names:?}");

    // FINDING (2026-05-01 audit): `check_db_setup` is NOT surfaced — neither
    // as call (it's not called) nor as reference (Python references
    // extraction lacks FnPointerArg / argument-position function-identifier
    // detection). This is a real indexer gap, NOT bench-tuning. The
    // dead_code.rs `*/checks.py + check_*` rule covers exactly this gap.
    //
    // Removal requires implementing Python FnPointerArg in
    // `crates/ga-parser/src/references.rs` — currently FnPointerArg is
    // documented for Rust only (references.rs:53-54).
    let surfaced = names.contains(&"check_db_setup") || call_names.contains(&"check_db_setup");
    assert!(
        !surfaced,
        "FINDING: ga-parser does NOT surface argument-position function refs in Python. \
         Removing dead_code.rs */checks.py rule requires Python FnPointerArg first. \
         Got refs={names:?} calls={call_names:?}"
    );
}

#[test]
fn audit_decorator_register_emits_reference() {
    // `@register()` decorator on `check_foo` — Django checks framework idiom.
    // Real question: does `check_foo` itself end up referenced?
    // Answer is structural: enclosing of decorator's call is module-level,
    // so `check_foo` (the decorated function) is just a definition. The
    // `@register()` call records `register` as callee; check_foo is not
    // an argument to that call.
    let src = "\
@register()
def check_foo():
    return 'ok'
";
    let cs = calls(src);
    let call_names: Vec<&str> = cs.iter().map(|c| c.callee_name.as_str()).collect();
    let rs = refs(src);
    let ref_names: Vec<&str> = rs.iter().map(|r| r.target_name.as_str()).collect();
    println!("[Q3] @register() check_foo calls: {call_names:?}");
    println!("[Q3] @register() check_foo refs: {ref_names:?}");

    // This test does NOT assert. It documents the gap: the only signal
    // emitted is `register` as callee. `check_foo` itself has no incoming
    // edge from this snippet — it's a definition with @register applied.
    // Django invokes check_foo via reflection on the registered function
    // object. Static analysis cannot see this without modeling `register`'s
    // semantics (i.e. that it stores the decorated callable in a registry).
    //
    // Conclusion: the */checks.py + check_* rule in dead_code.rs is
    // covering this exact gap. Removing it requires either:
    //   (a) framework-aware modeling of `register()`, or
    //   (b) accepting these as FPs.
}
