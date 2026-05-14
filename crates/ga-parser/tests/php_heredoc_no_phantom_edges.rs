//! v1.2-php S-001 AS-018 — heredoc / nowdoc body MUST NOT emit phantom edges.
//!
//! Security canary: tree-sitter-php tokenises heredoc complex interpolation
//! (`{$x->m()}`, `${func(...)}`, `{X::m()}`) as REAL call expression nodes
//! inside `heredoc_body`. A naive walker would treat those as call sites and
//! emit CALLS / REFERENCES edges from inside string content — graph
//! poisoning. AS-018 enforces that the PHP parser adapter suppresses
//! emission via an ancestor check on `heredoc_body` / `nowdoc_body`.
//!
//! Promoted from KG-PHP-3 "verify in fixture" to canary regression test on
//! 2026-05-13 per /mf-challenge Security finding M-3.

use std::fs;
use std::path::PathBuf;

use ga_core::Lang;
use ga_parser::{extract_calls, extract_references};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/php-tiny/symfony-mini/heredoc-cases.php")
}

fn load() -> Vec<u8> {
    fs::read(fixture_path()).expect("heredoc-cases.php fixture must exist")
}

/// Identifier names that appear ONLY inside heredoc / nowdoc bodies in the
/// fixture. If any of these surfaces in CALLS or REFERENCES, the walker
/// leaked into string content — graph poisoning.
const HEREDOC_ONLY_NAMES: &[&str] = &[
    "heredocPhantomMethod",
    "heredocPhantomFunction",
    "heredocPhantomStatic",
    "heredocPhantomClass",
    "heredocPhantomNew",
    "absorbAll",
    "shouldNotResolve",
    "staticPoison",
    "NowdocEvil",
    "callsiteShouldNotResolve",
    "statSiteShouldNotResolve",
    "HeredocDocstringEvil",
    "HeredocDocstringClass",
];

#[test]
fn extract_calls_does_not_leak_into_heredoc_body() {
    let src = load();
    let calls = extract_calls(Lang::Php, &src).expect("extract_calls Ok");

    let mut leaks: Vec<String> = Vec::new();
    for c in &calls {
        if HEREDOC_ONLY_NAMES.contains(&c.callee_name.as_str()) {
            leaks.push(format!(
                "callee_name='{}' at line {} — graph-poisoning leak",
                c.callee_name, c.call_site_line
            ));
        }
    }
    assert!(
        leaks.is_empty(),
        "AS-018 VIOLATION — phantom CALLS edges emitted from heredoc/nowdoc body:\n  {}\n\n\
         These identifiers exist ONLY inside heredoc-body / nowdoc-body string content. \
         If they reach extract_calls output, an attacker who controls a PHP file can plant \
         arbitrary fake CALLS edges in the graph DB by embedding `{{$x->method()}}` in a heredoc.",
        leaks.join("\n  ")
    );
}

#[test]
fn extract_references_does_not_leak_into_heredoc_body() {
    let src = load();
    let refs = extract_references(Lang::Php, &src).expect("extract_references Ok");

    let mut leaks: Vec<String> = Vec::new();
    for r in &refs {
        if HEREDOC_ONLY_NAMES.contains(&r.target_name.as_str()) {
            leaks.push(format!(
                "target_name='{}' at line {} — graph-poisoning leak",
                r.target_name, r.ref_site_line
            ));
        }
    }
    assert!(
        leaks.is_empty(),
        "AS-018 VIOLATION — phantom REFERENCES edges emitted from heredoc/nowdoc body:\n  {}",
        leaks.join("\n  ")
    );
}

#[test]
fn fixture_contains_expected_adversarial_shapes() {
    // Sanity guard: the fixture text must contain each adversarial shape we
    // claim to test. Otherwise the test above could be vacuously green
    // because the input doesn't actually try the attack.
    let src_str = String::from_utf8(load()).expect("fixture is utf-8");
    let required_markers = [
        "<<<PHP",
        "<<<'NOWDOC'",
        "<<<EOT",
        "<<<DOC",
        "heredocPhantomMethod",
        "${heredocPhantomFunction",
        "HeredocPhantomClass::heredocPhantomStatic",
        "absorbAll",
        "shouldNotResolve",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for m in required_markers {
        if !src_str.contains(m) {
            missing.push(m);
        }
    }
    assert!(
        missing.is_empty(),
        "heredoc-cases.php fixture missing adversarial markers: {missing:?}"
    );
}
