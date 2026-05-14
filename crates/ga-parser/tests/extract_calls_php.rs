//! v1.2-php S-001 AS-001 — PHP CALLS extraction (5 dispatch node-kinds) +
//! AS-005 parse tolerance.
//!
//! Node-kinds verified per docs/_archived/2026-05-13-php-node-kinds.md:
//! - `member_call_expression`             — `$x->m()`
//! - `function_call_expression`           — bare `f()`
//! - `nullsafe_member_call_expression`    — `$x?->m()` (PHP 8.0+)
//! - `scoped_call_expression`             — `X::m()` (PHP 5.3+)
//! - `object_creation_expression`         — `new X()`

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn member_call_expression_extracts_method_name_not_receiver() {
    // AS-001: $this->repo->findById($id) → callee = findById, NOT repo/this.
    let src = b"\
<?php
class UserService {
    public function getUser(int $id): ?User {
        return $this->repo->findById($id);
    }
}
";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    let find = calls
        .iter()
        .find(|c| c.callee_name == "findById")
        .unwrap_or_else(|| panic!("findById not found in calls: {calls:?}"));
    assert_eq!(find.enclosing_symbol.as_deref(), Some("getUser"));
}

#[test]
fn function_call_expression_extracts_bare_identifier() {
    // AS-001: bare function call strlen($s) → callee = strlen.
    let src = b"<?php function wrap(string $s): int { return strlen($s); }";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "strlen"),
        "strlen not in calls: {calls:?}"
    );
}

#[test]
fn nullsafe_member_call_expression_extracts_method_name() {
    // AS-001: $maybe?->findById($id) (PHP 8.0+) → callee = findById.
    let src = b"\
<?php
class Lookup {
    public function find(?UserRepository $maybe, int $id) {
        return $maybe?->findById($id);
    }
}
";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "findById"),
        "nullsafe call findById not emitted: {calls:?}"
    );
}

#[test]
fn scoped_call_expression_extracts_method_name_strips_class_receiver() {
    // AS-001: Cache::warm($user) → callee = warm. Class `Cache` is receiver,
    // never appears as callee_name (would over-report calls).
    let src = b"\
<?php
class Service {
    public function go() {
        Cache::warm($x);
    }
}
";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "warm"),
        "scoped call warm not emitted: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.callee_name == "Cache"),
        "receiver Cache leaked as callee: {calls:?}"
    );
}

#[test]
fn object_creation_expression_extracts_class_name_skipping_new_token() {
    // AS-001: new User(...) → callee = User. The `new` token must not leak.
    let src = b"\
<?php
class Factory {
    public function make() {
        return new User(1, 'jane');
    }
}
";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "User"),
        "object_creation_expression `new User()` must emit User: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.callee_name == "new"),
        "`new` keyword leaked as callee: {calls:?}"
    );
}

#[test]
fn object_creation_with_qualified_name_strips_namespace_prefix() {
    // `new \App\Entity\User()` → callee = User (trailing identifier).
    let src = b"\
<?php
class Factory {
    public function make() {
        return new \\App\\Entity\\User(1, 'jane');
    }
}
";
    let calls = extract_calls(Lang::Php, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "User"),
        "qualified `new \\App\\Entity\\User()` must surface User: {calls:?}"
    );
}

#[test]
fn malformed_php_returns_ok_with_partial_calls() {
    // AS-005: parse tolerance per R12. Source has a valid method + broken one.
    // Walker must extract from the valid portion and not panic.
    let src = b"\
<?php
class Partial {
    public function valid(): int {
        return strlen('ok');
    }
    public function broken(: void {
";
    let result = extract_calls(Lang::Php, src);
    assert!(
        result.is_ok(),
        "extract_calls must not panic on malformed PHP"
    );
    let calls = result.unwrap();
    // The valid portion's strlen() must surface even though the trailing fn is broken.
    assert!(
        calls.iter().any(|c| c.callee_name == "strlen"),
        "valid pre-broken portion must still emit: {calls:?}"
    );
}
