//! v1.1-M4 S-004b — Ruby CALLS extraction (AS-012).
//!
//! Pin the per-AS contract for `extract_calls(Lang::Ruby, ...)`. AS-012
//! happy path: bare-call `check(name)` inside a method emits a
//! ParsedCall with enclosing == method-name. Per-lang CALLEE_EXTRACTOR
//! handles the `call` node's `method` field (default falls back to
//! `child(0)` which is the receiver token — wrong for `Base.lookup(...)`).

use ga_core::Lang;
use ga_parser::extract_calls;

fn calls(src: &str) -> Vec<ga_parser::ParsedCall> {
    extract_calls(Lang::Ruby, src.as_bytes()).expect("extract_calls Ok")
}

// ─────────────────────────────────────────────────────────────────────────
// AS-012 happy path + canonical receiver / qualified / chained forms
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn bare_call_inside_method_emits_parsed_call_with_enclosing() {
    // AS-012 canonical Given/Then.
    let src = "\
class User\n\
  def authenticate(pw)\n\
    check(pw)\n\
  end\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.callee_name == "check")
        .expect("expected callee `check` from authenticate");
    assert_eq!(hit.enclosing_symbol.as_deref(), Some("authenticate"));
    assert_eq!(hit.call_site_line, 3);
}

#[test]
fn receiver_call_emits_method_name_not_receiver() {
    // Default extractor falls back to child(0) which would emit "Base"
    // (the receiver) — must instead emit "lookup" (the method).
    let src = "\
def find(id)\n\
  Base.lookup(id)\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.enclosing_symbol.as_deref() == Some("find"))
        .expect("call inside `find`");
    assert_eq!(
        hit.callee_name, "lookup",
        "method name (lookup), NOT receiver (Base)"
    );
    // Negative — Base must NOT leak as a callee from this call site.
    assert!(
        !out.iter().any(|c| c.callee_name == "Base"),
        "receiver `Base` must not leak as a callee_name"
    );
}

#[test]
fn instance_receiver_call_emits_method_name() {
    // `obj.process` (lowercase identifier receiver, not constant).
    let src = "\
def run(obj)\n\
  obj.process(42)\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.enclosing_symbol.as_deref() == Some("run"))
        .expect("call inside run");
    assert_eq!(hit.callee_name, "process");
}

#[test]
fn chained_call_emits_each_method_name() {
    // `a.b.c` — tree-sitter-ruby nests inner call inside outer's receiver.
    // The outer call's method is `c`; the inner call is the receiver and
    // is also walked → its method is `b`. We expect both to surface.
    let src = "\
def fetch\n\
  client.get(\"/users\").parse\n\
end\n";
    let out = calls(src);
    let names: Vec<&str> = out
        .iter()
        .filter(|c| c.enclosing_symbol.as_deref() == Some("fetch"))
        .map(|c| c.callee_name.as_str())
        .collect();
    assert!(names.contains(&"parse"), "outer .parse: got {names:?}");
    assert!(names.contains(&"get"), "inner .get: got {names:?}");
    assert!(
        !names.contains(&"client"),
        "receiver `client` must not leak"
    );
}

#[test]
fn parenless_call_inside_method_emits_callee() {
    // Ruby allows parenless calls — `puts foo` parses as `call` with a
    // bare identifier child(0) and no `method` field. Fallback path.
    let src = "\
def announce\n\
  puts \"hello\"\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.enclosing_symbol.as_deref() == Some("announce"))
        .expect("call inside announce");
    assert_eq!(hit.callee_name, "puts");
}

#[test]
fn singleton_method_enclosing_resolved_to_method_name() {
    // `def self.find(id) ... end` — singleton_method node — enclosing
    // resolves to the method's name field per the walker's name_from_node.
    let src = "\
class Repo\n\
  def self.find(id)\n\
    Base.lookup(id)\n\
  end\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.callee_name == "lookup")
        .expect("lookup callee");
    assert_eq!(
        hit.enclosing_symbol.as_deref(),
        Some("find"),
        "singleton_method enclosing should be the method short name"
    );
}

#[test]
fn module_function_call_resolves_to_method_name() {
    // Module-level methods (Rails: `Foo.bar` style helpers) compose with
    // module + class enclosing scopes. We only assert the callee_name
    // resolution + non-empty enclosing — the typed scope variant is a
    // walker concern, not an extract_calls one.
    let src = "\
module App\n\
  class User < Base\n\
    def initialize(name)\n\
      @name = name\n\
      check(name)\n\
    end\n\
  end\n\
end\n";
    let out = calls(src);
    let hit = out
        .iter()
        .find(|c| c.callee_name == "check")
        .expect("check callee");
    assert_eq!(hit.enclosing_symbol.as_deref(), Some("initialize"));
}

// ─────────────────────────────────────────────────────────────────────────
// AS-005-equiv parse tolerance / R12 panic-safety
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_source_returns_ok_empty() {
    let out = calls("");
    assert!(out.is_empty(), "empty source → no calls");
}

#[test]
fn malformed_ruby_source_returns_ok_with_partial_parse() {
    // R12: parse-tolerance — malformed Ruby (unclosed `def`, missing `end`)
    // must still return Ok. Tree-sitter emits ERROR nodes but the call
    // before the ERROR boundary should still surface.
    let src = "\
class User\n\
  def authenticate(pw\n\
    check(pw)\n\
";
    let result = extract_calls(Lang::Ruby, src.as_bytes());
    assert!(
        result.is_ok(),
        "AS-005-equiv: malformed Ruby must NOT error, got: {:?}",
        result.err()
    );
}

#[test]
fn garbage_bytes_does_not_panic() {
    // Defense-in-depth: arbitrary bytes must not panic the extractor.
    let garbage: &[u8] = &[0x00, 0xff, 0xfe, 0x7f, 0x80, 0xc0, 0xc1];
    let result = std::panic::catch_unwind(|| extract_calls(Lang::Ruby, garbage));
    assert!(
        result.is_ok(),
        "extract_calls panicked on garbage Ruby bytes"
    );
}
