// Probe tree-sitter-rust use_declaration AST shape.
use tree_sitter::Parser;

fn print(node: tree_sitter::Node, depth: usize, src: &[u8]) {
    let text = node.utf8_text(src).unwrap_or("?");
    let snippet = if text.len() > 40 { &text[..40] } else { text };
    println!(
        "{}{} [{}..{}] '{}'",
        "  ".repeat(depth),
        node.kind(),
        node.start_byte(),
        node.end_byte(),
        snippet.replace('\n', "\\n")
    );
    let mut c = node.walk();
    for ch in node.named_children(&mut c) {
        print(ch, depth + 1, src);
    }
}

fn main() {
    let mut p = Parser::new();
    p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
    for src in [
        "use foo::Bar;\n",
        "use foo::Bar as B;\n",
        "use foo::{Bar, Baz};\n",
        "use foo::*;\n",
        "use foo;\n",
        "use foo::{Bar, Baz as Z, sub::*};\n",
        "use crate::a::b::Foo;\n",
    ] {
        println!("=== {:?} ===", src);
        let tree = p.parse(src.as_bytes(), None).unwrap();
        print(tree.root_node(), 0, src.as_bytes());
    }
}
