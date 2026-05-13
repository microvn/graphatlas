//! Dump tree-sitter-c-sharp + tree-sitter-kotlin-ng AST for return-type
//! field discovery (Gap 3).

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
        snippet.chars().take(80).collect::<String>()
    );
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_ast(&child, source, depth + 1);
    }
}

fn main() {
    println!("=== C# ===");
    let src_cs =
        b"class F { public int Add(int a, int b) { return a + b; } public void None() {} }\n";
    let mut p = Parser::new();
    let lang_cs: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    p.set_language(&lang_cs).unwrap();
    let tree = p.parse(src_cs, None).unwrap();
    print_ast(&tree.root_node(), src_cs, 0);

    println!("\n=== Kotlin ===");
    let src_kt = b"package x\nopen class Base { open fun greet(): String { return \"hi\" } }\nclass Child : Base() { override fun greet(): String { return \"hi from child\" } }\n";
    let mut p2 = Parser::new();
    let lang_kt: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
    p2.set_language(&lang_kt).unwrap();
    let tree2 = p2.parse(src_kt, None).unwrap();
    print_ast(&tree2.root_node(), src_kt, 0);
}
