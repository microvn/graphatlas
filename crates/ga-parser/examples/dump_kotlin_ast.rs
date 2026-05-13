//! PR5c2b debug — dump tree-sitter-kotlin AST for a fn to find the
//! parameter-container kind/field name.

use tree_sitter::Parser;

fn print_ast(node: &tree_sitter::Node, source: &[u8], depth: usize) {
    let pad = "  ".repeat(depth);
    let kind = node.kind();
    let snippet = node
        .utf8_text(source)
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    println!(
        "{pad}{kind} [named={}] '{}'",
        node.is_named(),
        snippet.chars().take(60).collect::<String>()
    );
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_ast(&child, source, depth + 1);
    }
}

fn main() {
    let src = b"package x\nfun do_work(input: Int): Int { return input }\n";
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    print_ast(&tree.root_node(), src, 0);
}
