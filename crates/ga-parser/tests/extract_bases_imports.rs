//! AS-010: extract_bases() + extract_import_path() end-to-end per language.
//! Ported assertions mirror rust-poc/src/main.rs behavior.

use ga_core::Lang;
use ga_parser::ParserPool;
use tree_sitter::Parser;

fn parse(lang: Lang, src: &[u8]) -> (Parser, tree_sitter::Tree, &[u8]) {
    let pool = ParserPool::new();
    let spec = pool.spec_for(lang).unwrap();
    let mut parser = Parser::new();
    parser.set_language(&spec.tree_sitter_lang()).unwrap();
    let tree = parser.parse(src, None).unwrap();
    (parser, tree, src)
}

fn find_first<'tree>(
    tree: &'tree tree_sitter::Tree,
    kinds: &[&str],
) -> Option<tree_sitter::Node<'tree>> {
    fn walk<'t>(node: tree_sitter::Node<'t>, kinds: &[&str]) -> Option<tree_sitter::Node<'t>> {
        if kinds.contains(&node.kind()) {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(hit) = walk(child, kinds) {
                return Some(hit);
            }
        }
        None
    }
    walk(tree.root_node(), kinds)
}

// --- extract_bases --------------------------------------------------------

#[test]
fn python_extract_bases_finds_superclasses() {
    let src = b"class Dog(Animal, Serializable):\n    pass\n";
    let (_p, tree, source) = parse(Lang::Python, src);
    let node = find_first(&tree, &["class_definition"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Python)
        .unwrap()
        .extract_bases(&node, source);
    assert!(bases.contains(&"Animal".to_string()), "{bases:?}");
    assert!(bases.contains(&"Serializable".to_string()), "{bases:?}");
}

#[test]
fn python_extract_bases_strips_module_prefix() {
    let src = b"class UserApi(pkg.Base):\n    pass\n";
    let (_p, tree, source) = parse(Lang::Python, src);
    let node = find_first(&tree, &["class_definition"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Python)
        .unwrap()
        .extract_bases(&node, source);
    assert_eq!(
        bases,
        vec!["Base".to_string()],
        "pkg. prefix should be stripped"
    );
}

#[test]
fn typescript_extract_bases_extends_clause() {
    let src = b"class Dog extends Animal implements Trainable {}\n";
    let (_p, tree, source) = parse(Lang::TypeScript, src);
    let node = find_first(&tree, &["class_declaration"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::TypeScript)
        .unwrap()
        .extract_bases(&node, source);
    assert!(bases.contains(&"Animal".to_string()), "{bases:?}");
    assert!(bases.contains(&"Trainable".to_string()), "{bases:?}");
}

#[test]
fn rust_extract_bases_trait_impl() {
    let src = b"impl Greet for User {}\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["impl_item"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_bases(&node, source);
    assert_eq!(bases, vec!["Greet".to_string()], "{bases:?}");
}

#[test]
fn rust_extract_bases_inherent_impl_empty() {
    // `impl User {}` — no trait bound → no bases.
    let src = b"impl User { fn new() {} }\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["impl_item"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_bases(&node, source);
    assert!(bases.is_empty(), "{bases:?}");
}

#[test]
fn rust_extract_bases_strips_generics() {
    let src = b"impl<T> Greet<T> for User<T> {}\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["impl_item"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_bases(&node, source);
    assert_eq!(bases, vec!["Greet".to_string()], "{bases:?}");
}

#[test]
fn go_extract_bases_always_empty() {
    let src = b"package main\ntype X struct{}\n";
    let (_p, tree, source) = parse(Lang::Go, src);
    let node = find_first(&tree, &["type_spec"]).unwrap();
    let pool = ParserPool::new();
    let bases = pool
        .spec_for(Lang::Go)
        .unwrap()
        .extract_bases(&node, source);
    assert!(bases.is_empty());
}

// --- extract_import_path --------------------------------------------------

#[test]
fn python_import_from_path() {
    let src = b"from pkg.sub import foo\n";
    let (_p, tree, source) = parse(Lang::Python, src);
    let node = find_first(&tree, &["import_from_statement"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::Python)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(path, Some("pkg.sub".to_string()));
}

#[test]
fn typescript_import_literal() {
    let src = b"import { x } from './utils';\n";
    let (_p, tree, source) = parse(Lang::TypeScript, src);
    let node = find_first(&tree, &["import_statement"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::TypeScript)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(path, Some("./utils".to_string()));
}

#[test]
fn javascript_import_literal() {
    let src = b"import react from 'react';\n";
    let (_p, tree, source) = parse(Lang::JavaScript, src);
    let node = find_first(&tree, &["import_statement"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::JavaScript)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(path, Some("react".to_string()));
}

#[test]
fn rust_extract_import_path_returns_none_for_non_use_node() {
    // Regression: crates/ga-parser/src/langs/rs.rs:62-75 fallback branch
    // returned raw utf8_text for any node — e.g. passing a `mod` item yielded
    // the whole body as the "import path". Must now return None.
    let src = b"mod foo { pub fn bar() {} }\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["mod_item"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(
        path, None,
        "non-use node must not produce an import path; got {path:?}"
    );
}

#[test]
fn rust_extract_import_path_returns_none_for_function_item() {
    let src = b"fn greet() {}\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["function_item"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(path, None);
}

#[test]
fn rust_extract_import_path_still_works_on_use_declaration() {
    // Happy path must keep working post-fix.
    let src = b"use std::collections::HashMap;\n";
    let (_p, tree, source) = parse(Lang::Rust, src);
    let node = find_first(&tree, &["use_declaration"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::Rust)
        .unwrap()
        .extract_import_path(&node, source);
    assert!(path.is_some(), "use_declaration should yield Some");
    let p = path.unwrap();
    // Either the shared helper caught a scoped_identifier, or we fell back
    // to the raw text. Either way it must contain the actual path.
    assert!(p.contains("std"), "path: {p}");
    assert!(p.contains("HashMap"), "path: {p}");
}

#[test]
fn go_import_literal() {
    let src = b"package main\nimport \"fmt\"\n";
    let (_p, tree, source) = parse(Lang::Go, src);
    let node = find_first(&tree, &["import_declaration"]).unwrap();
    let pool = ParserPool::new();
    let path = pool
        .spec_for(Lang::Go)
        .unwrap()
        .extract_import_path(&node, source);
    assert_eq!(path, Some("fmt".to_string()));
}
