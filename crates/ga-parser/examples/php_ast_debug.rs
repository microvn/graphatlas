use tree_sitter::Parser;

fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
    let indent = "  ".repeat(depth);
    let text = node.utf8_text(src).unwrap_or("?");
    let snippet: String = text.chars().take(60).collect();
    eprintln!(
        "{}{} named={} : {:?}",
        indent,
        node.kind(),
        node.is_named(),
        snippet
    );
    let mut c = node.walk();
    for child in node.children(&mut c) {
        dump(child, src, depth + 1);
    }
}

fn main() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .unwrap();
    let src = "<?php\nclass C {\n    #[Required]\n    public UserRepository $repo;\n}\n";
    eprintln!("=== {} ===", src);
    let tree = parser.parse(src, None).unwrap();
    dump(tree.root_node(), src.as_bytes(), 0);
}
