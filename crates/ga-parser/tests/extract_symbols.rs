//! S-004 AS-010 — LanguageSpec trait + 5 impls extract symbols per lang.
//! Ported from rust-poc/src/main.rs:234-400 (predicates) + :956 (walker).

use ga_core::{Lang, SymbolKind};
use ga_parser::parse_source;

// --- Python ---------------------------------------------------------------

#[test]
fn python_function_and_class() {
    let src = r#"
def greet(name):
    return f"hi {name}"

class User:
    def __init__(self, name):
        self.name = name
"#;
    let syms = parse_source(Lang::Python, src.as_bytes()).expect("parse");
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"), "expected greet in {names:?}");
    assert!(names.contains(&"User"), "expected User in {names:?}");
    assert!(
        names.contains(&"__init__"),
        "expected __init__ in {names:?}"
    );

    let user = syms.iter().find(|s| s.name == "User").unwrap();
    assert_eq!(user.kind, SymbolKind::Class);
    let greet = syms.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(greet.kind, SymbolKind::Function);
}

#[test]
fn python_nested_function() {
    let src = r#"
def outer():
    def inner():
        return 1
    return inner
"#;
    let syms = parse_source(Lang::Python, src.as_bytes()).unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"outer"));
    assert!(names.contains(&"inner"));
}

// --- Rust -----------------------------------------------------------------

#[test]
fn rust_items_cover_multiple_kinds() {
    let src = r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }

pub struct User { name: String }

impl User {
    pub fn new(name: String) -> Self { Self { name } }
}

pub trait Greet { fn hello(&self); }

pub enum State { On, Off }
"#;
    let syms = parse_source(Lang::Rust, src.as_bytes()).unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    for expected in ["add", "User", "new", "Greet", "State"] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
}

// --- TypeScript -----------------------------------------------------------

#[test]
fn typescript_function_class_arrow() {
    let src = r#"
export function greet(name: string): string {
    return `hi ${name}`;
}

export class Service {
    run(): void {}
}

export const helper = (x: number) => x * 2;
"#;
    let syms = parse_source(Lang::TypeScript, src.as_bytes()).unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"), "missing greet: {names:?}");
    assert!(names.contains(&"Service"), "missing Service: {names:?}");
    assert!(names.contains(&"run"), "missing run method: {names:?}");
}

// --- JavaScript -----------------------------------------------------------

#[test]
fn javascript_function_class() {
    let src = r#"
function greet(name) {
    return `hi ${name}`;
}

class Service {
    run() {}
}
"#;
    let syms = parse_source(Lang::JavaScript, src.as_bytes()).unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"greet"));
    assert!(names.contains(&"Service"));
}

// --- Go -------------------------------------------------------------------

#[test]
fn go_function_and_method() {
    let src = r#"
package main

type User struct {
    Name string
}

func (u *User) Greet() string {
    return "hi"
}

func main() {}
"#;
    let syms = parse_source(Lang::Go, src.as_bytes()).unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"User"), "missing User type: {names:?}");
    assert!(names.contains(&"Greet"), "missing Greet method: {names:?}");
    assert!(names.contains(&"main"), "missing main func: {names:?}");
}

// --- Edge cases -----------------------------------------------------------

#[test]
fn empty_source_produces_no_symbols() {
    for lang in [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
    ] {
        let syms = parse_source(lang, b"").unwrap();
        assert!(syms.is_empty(), "{lang:?} empty source returned {syms:?}");
    }
}

#[test]
fn symbol_line_numbers_are_1_based() {
    let src = "\n\ndef foo():\n    pass\n";
    let syms = parse_source(Lang::Python, src.as_bytes()).unwrap();
    let foo = syms.iter().find(|s| s.name == "foo").expect("found foo");
    assert_eq!(foo.line, 3, "foo should be on line 3, got {}", foo.line);
}

#[test]
fn parser_pool_registers_all_v1_plus_phase_c_langs() {
    // v1 shipped 5 langs (Python/TS/JS/Go/Rust); v1.1-M4 phase C added
    // Java (S-001), Kotlin (S-002), CSharp (S-003), Ruby (S-004a).
    // v1.2 S-001 adds Php. Total = 10 — matches Lang::ALL.len().
    use ga_parser::ParserPool;
    let pool = ParserPool::new();
    assert_eq!(pool.registered_langs().len(), 10);
    for lang in [
        Lang::Python,
        Lang::TypeScript,
        Lang::JavaScript,
        Lang::Go,
        Lang::Rust,
        Lang::Java,
        Lang::Kotlin,
        Lang::CSharp,
        Lang::Ruby,
        Lang::Php,
    ] {
        assert!(
            pool.spec_for(lang).is_some(),
            "pool missing impl for {lang:?}"
        );
    }
}
