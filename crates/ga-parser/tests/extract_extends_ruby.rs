//! v1.1-M4 S-004c — Ruby EXTENDS extraction (AS-013-equiv).
//!
//! Spec contract: `class X < Y` emits one EXTENDS edge X → Y. Ruby has no
//! multiple-inheritance form (mixins via `include` / `extend` are method
//! calls, not structural inheritance). Qualified bases via
//! `scope_resolution` (`Foo::Bar`) are stripped to the trailing constant.

use ga_core::Lang;
use ga_parser::extract_extends;

fn extends(src: &str) -> Vec<ga_parser::ParsedExtends> {
    extract_extends(Lang::Ruby, src.as_bytes()).expect("extract_extends Ok")
}

#[test]
fn class_with_constant_superclass_emits_extends_edge() {
    let out = extends("class Admin < User\nend\n");
    assert_eq!(out.len(), 1, "exactly one EXTENDS edge");
    assert_eq!(out[0].class_name, "Admin");
    assert_eq!(out[0].base_name, "User");
}

#[test]
fn class_with_scope_resolution_superclass_strips_to_trailing_constant() {
    // `class X < Foo::Bar` → base = "Bar" (last segment)
    let out = extends("class App < Foo::Bar\nend\n");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].class_name, "App");
    assert_eq!(
        out[0].base_name, "Bar",
        "namespace stripped, trailing constant"
    );
}

#[test]
fn class_with_deeply_qualified_superclass_strips_to_last_segment() {
    // Rails idiom: `class UsersController < ApplicationController::Base`
    let out = extends("class UsersController < ActionController::API::Base\nend\n");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].base_name, "Base");
}

#[test]
fn class_without_superclass_emits_no_edge() {
    let out = extends("class Foo\nend\n");
    assert!(out.is_empty(), "no superclass → no EXTENDS edge");
}

#[test]
fn module_does_not_emit_extends_edge() {
    // Ruby `module Foo` has no superclass relation.
    let out = extends("module App\nend\n");
    assert!(out.is_empty(), "module is not a class — no EXTENDS");
}

#[test]
fn multiple_classes_in_same_file_each_emit_their_own_edge() {
    let src = "\
class A < Base\nend\n\
class B < Foo::Bar\nend\n\
class C\nend\n\
class D < ApplicationController::Base\nend\n";
    let out = extends(src);
    assert_eq!(out.len(), 3, "A, B, D have superclasses; C does not");
    let by_subject: std::collections::HashMap<&str, &str> = out
        .iter()
        .map(|e| (e.class_name.as_str(), e.base_name.as_str()))
        .collect();
    assert_eq!(by_subject.get("A"), Some(&"Base"));
    assert_eq!(by_subject.get("B"), Some(&"Bar"));
    assert_eq!(by_subject.get("D"), Some(&"Base"));
    assert!(!by_subject.contains_key("C"));
}

#[test]
fn nested_class_inside_module_still_emits_edge() {
    let src = "\
module App\n\
  class User < Base\n\
  end\n\
end\n";
    let out = extends(src);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].class_name, "User");
    assert_eq!(out[0].base_name, "Base");
}

#[test]
fn malformed_ruby_does_not_panic() {
    // R12: panic-safety on garbage byte streams.
    let garbage: &[u8] = &[0x00, 0xff, 0xfe, 0x7f, 0x80];
    let result = std::panic::catch_unwind(|| extract_extends(Lang::Ruby, garbage));
    assert!(result.is_ok(), "extract_extends panicked on garbage bytes");
}

#[test]
fn empty_source_returns_ok_empty() {
    assert!(extends("").is_empty());
}

#[test]
fn include_module_does_not_count_as_extends() {
    // Mixin via `include` is a method call, not structural inheritance.
    // Parser-layer correctly returns empty; mixin resolution is a REFERENCES
    // / indexer-layer concern (out of scope for extract_extends).
    let src = "\
class Foo\n\
  include Bar\n\
end\n";
    let out = extends(src);
    assert!(
        out.is_empty(),
        "include is a method call, not class<superclass — got {out:?}"
    );
}
