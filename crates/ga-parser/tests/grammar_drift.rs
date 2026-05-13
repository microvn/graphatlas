//! AS-010 "AST-node-kinds checklist validated against pinned tree-sitter-<lang>
//! grammar SHA in Cargo.lock". Each const kind listed in langs/*.rs must still
//! be emitted by the current pinned grammar. If a grammar bump silently
//! renames a node kind (e.g. `function_definition` → `function_def`), this
//! test turns red with a precise diff — catching drift at CI, not prod.
//!
//! v1.1-M4 (S-005a D6) — also satisfies **AS-016 grammar SHA pinning
//! regression guard** (graphatlas-v1.1-languages.md). Static pin
//! enforcement: `cargo_pin_strict.rs`. Dynamic drift detection: this file.
//!
//! Methodology: parse a fixture source per lang that exercises every symbol /
//! import / call / extends node kind, collect the set of actual node kinds
//! seen, and assert every const in the lang's list appears in that set.

use ga_core::Lang;
use ga_parser::ParserPool;
use std::collections::HashSet;
use tree_sitter::{Node, Parser, Tree};

/// Source that exercises every symbol / import / call / extends node kind for
/// a given language. If you add a const to a lang's `*_node_kinds()` list,
/// add a snippet here that produces that kind — otherwise the drift test
/// will flag it as "unused".
fn exhaustive_source(lang: Lang) -> &'static str {
    match lang {
        Lang::Python => {
            "\
import os\n\
from pkg import thing\n\
@decorator\n\
def f():\n    g()\n\
class C(Base):\n    def m(self): self.f()\n"
        }
        Lang::TypeScript => {
            "\
import { x } from 'm';\n\
interface I { run(): void; }\n\
function f(): void {}\n\
class C extends B { run(): void { new X(); f(); } }\n\
const h = (n: number) => n + 1;\n"
        }
        Lang::JavaScript => {
            "\
import x from 'm';\n\
function f() {}\n\
class C extends B { run() { new X(); f(); } }\n\
const h = (n) => n + 1;\n\
const jsx = <Foo bar=\"baz\" />;\n\
const jsx2 = <Foo>hello</Foo>;\n"
        }
        Lang::Go => {
            "\
package main\n\
import \"fmt\"\n\
type U struct { N string }\n\
func (u *U) G() { fmt.Println(\"\") }\n\
func main() { G() }\n"
        }
        Lang::Rust => {
            "\
use std::io;\n\
fn f() { g(); println!(); }\n\
struct S;\n\
enum E { A }\n\
trait T { fn x(&self); }\n\
impl T for S { fn x(&self) {} }\n"
        }
        Lang::Java => {
            // Exercises every kind in `langs::java::JavaLang::*_node_kinds`:
            // class_declaration / interface_declaration / enum_declaration /
            // method_declaration / constructor_declaration / import_declaration /
            // method_invocation / object_creation_expression.
            "\
package com.example;\n\
import java.util.List;\n\
public interface Printable { void print(); }\n\
public enum Status { ACTIVE, INACTIVE }\n\
public class User { String name; }\n\
public class Admin extends User implements Printable {\n\
    private final Status s;\n\
    public Admin(Status s) { this.s = s; new User(); }\n\
    public void print() { helper(); }\n\
    void helper() {}\n\
}\n"
        }
        Lang::Kotlin => {
            // Exercises every kind in `langs::kotlin::KotlinLang::*_node_kinds`:
            // class_declaration + object_declaration + function_declaration +
            // import + call_expression.
            //
            // Single-container layout — tree-sitter-kotlin-ng 1.1 errors on
            // multiple sibling top-level type declarations (real Kotlin
            // allows them, grammar limitation). Wrapping siblings inside one
            // outer `class` body parses cleanly. The AS-010 contract is
            // "every kind listed in *_node_kinds must appear in some parse",
            // not "exhaustive grammar coverage".
            "\
package com.example\n\
import org.foo.Bar\n\
class Container {\n\
    object Inner {\n\
        fun foo() = bar()\n\
    }\n\
    fun helper() = baz()\n\
}\n"
        }
        Lang::CSharp => {
            // Exercises every kind in `langs::csharp::CSharpLang::*_node_kinds`:
            // class_declaration / interface_declaration / enum_declaration /
            // struct_declaration / record_declaration / delegate_declaration /
            // method_declaration / constructor_declaration / using_directive /
            // invocation_expression / object_creation_expression.
            //
            // Single namespace block keeps top-level declarations scoped
            // (avoids any tree-sitter-c-sharp top-level quirks similar to
            // the Kotlin multi-decl issue).
            "\
using System;\n\
namespace App {\n\
    public interface IPrintable { void Print(); }\n\
    public enum Status { Active, Inactive }\n\
    public struct Point { public int X; }\n\
    public record Pair(int A, int B);\n\
    public delegate int Handler(string s);\n\
    public class User { public User(string n) {} public void Greet() { Console.WriteLine(\"hi\"); } }\n\
    public class Admin : User { public Admin() : base(\"a\") { var u = new User(\"b\"); } }\n\
}\n"
        }
        Lang::Ruby => {
            // Exercises every kind in `langs::ruby::RubyLang::*_node_kinds`:
            // class / module / method / singleton_method / call.
            //
            // Ruby IMPORTS list is intentionally empty (require/require_relative
            // are runtime method calls, not structural import nodes — see
            // langs/ruby.rs IMPORTS). The drift loop iterates `import_node_kinds`
            // and finds nothing → vacuously satisfied.
            "\
module App\n\
  class User < Base\n\
    def initialize(name)\n\
      check(name)\n\
    end\n\
    def self.find(id)\n\
      Base.lookup(id)\n\
    end\n\
  end\n\
end\n"
        }
    }
}

fn collect_node_kinds<'t>(node: Node<'t>, out: &mut HashSet<&'t str>) {
    out.insert(node.kind());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_kinds(child, out);
    }
}

fn parse_and_collect(lang: Lang) -> Tree {
    let pool = ParserPool::new();
    let spec = pool.spec_for(lang).unwrap();
    let mut parser = Parser::new();
    parser.set_language(&spec.tree_sitter_lang()).unwrap();
    parser
        .parse(exhaustive_source(lang).as_bytes(), None)
        .unwrap()
}

fn check_lang(lang: Lang) {
    let tree = parse_and_collect(lang);
    let mut kinds: HashSet<&str> = HashSet::new();
    collect_node_kinds(tree.root_node(), &mut kinds);

    let pool = ParserPool::new();
    let spec = pool.spec_for(lang).unwrap();

    for category in [
        ("symbol", spec.symbol_node_kinds()),
        ("import", spec.import_node_kinds()),
        ("call", spec.call_node_kinds()),
        ("extends", spec.extends_node_kinds()),
    ] {
        let (label, list) = category;
        for &k in list {
            assert!(
                kinds.contains(k),
                "{lang:?}: {label}_node_kinds lists `{k}` but the pinned grammar \
                 never emits it in the exhaustive fixture. Either the grammar \
                 renamed the node (AS-010 drift) or the fixture is missing a \
                 snippet that produces this kind. Seen kinds sample: {:?}",
                kinds.iter().take(20).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn python_grammar_kinds_still_emitted() {
    check_lang(Lang::Python);
}

#[test]
fn typescript_grammar_kinds_still_emitted() {
    check_lang(Lang::TypeScript);
}

#[test]
fn javascript_grammar_kinds_still_emitted() {
    check_lang(Lang::JavaScript);
}

#[test]
fn go_grammar_kinds_still_emitted() {
    check_lang(Lang::Go);
}

#[test]
fn rust_grammar_kinds_still_emitted() {
    check_lang(Lang::Rust);
}

#[test]
fn java_grammar_kinds_still_emitted() {
    check_lang(Lang::Java);
}

#[test]
fn kotlin_grammar_kinds_still_emitted() {
    check_lang(Lang::Kotlin);
}

#[test]
fn csharp_grammar_kinds_still_emitted() {
    check_lang(Lang::CSharp);
}

#[test]
fn ruby_grammar_kinds_still_emitted() {
    check_lang(Lang::Ruby);
}
