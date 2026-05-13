//! v1.1-M4 S-003c — C# AS-011 partial classes (parser-side contract).
//!
//! AS-011 (C# partial classes):
//!   Given: `partial class Foo` across 2 files
//!   When:  Indexed
//!   Then:  Both files linked to same Symbol node (merge by FQN). The
//!          PARSER side emits one ParsedSymbol per file with name `Foo`;
//!          the INDEXER (downstream of ga-parser) is responsible for
//!          merging by FQN. This test pins the parser-side contract.
//!
//! Also pins symbol-shape baseline for the other distinctive C# kinds
//! that S-003a registered (record / delegate / struct / interface) so a
//! grammar bump that renames them would surface here, not silently at
//! M2 gate time.

use ga_core::{Lang, SymbolKind};
use ga_parser::parse_source;

#[test]
fn partial_class_two_files_each_emits_symbol_with_same_name() {
    // File A.
    let src_a =
        b"namespace App {\n    partial class Foo {\n        public void MethodA() {}\n    }\n}\n";
    let syms_a = parse_source(Lang::CSharp, src_a).expect("parse_source Ok A");
    let foo_a = syms_a
        .iter()
        .find(|s| s.name == "Foo" && s.kind == SymbolKind::Class)
        .unwrap_or_else(|| panic!("File A: partial class Foo missing: {syms_a:?}"));
    assert_eq!(foo_a.name, "Foo");

    // File B.
    let src_b =
        b"namespace App {\n    partial class Foo {\n        public void MethodB() {}\n    }\n}\n";
    let syms_b = parse_source(Lang::CSharp, src_b).expect("parse_source Ok B");
    let foo_b = syms_b
        .iter()
        .find(|s| s.name == "Foo" && s.kind == SymbolKind::Class)
        .unwrap_or_else(|| panic!("File B: partial class Foo missing: {syms_b:?}"));
    assert_eq!(foo_b.name, "Foo");

    // Both files surface their methods scoped to the partial class.
    assert!(
        syms_a.iter().any(|s| s.name == "MethodA"),
        "File A: MethodA must surface as a symbol: {syms_a:?}"
    );
    assert!(
        syms_b.iter().any(|s| s.name == "MethodB"),
        "File B: MethodB must surface as a symbol: {syms_b:?}"
    );
}

#[test]
fn record_declaration_emits_symbol() {
    let src = b"public record Point(int X, int Y);\n";
    let syms = parse_source(Lang::CSharp, src).expect("parse_source Ok");
    assert!(
        syms.iter().any(|s| s.name == "Point"),
        "record declaration must emit a symbol: {syms:?}"
    );
}

#[test]
fn delegate_declaration_emits_symbol() {
    let src = b"public delegate int Handler(string s);\n";
    let syms = parse_source(Lang::CSharp, src).expect("parse_source Ok");
    assert!(
        syms.iter().any(|s| s.name == "Handler"),
        "delegate declaration must emit a symbol: {syms:?}"
    );
}

#[test]
fn struct_declaration_emits_symbol_with_struct_kind() {
    let src = b"public struct Point { public int X; }\n";
    let syms = parse_source(Lang::CSharp, src).expect("parse_source Ok");
    let point = syms
        .iter()
        .find(|s| s.name == "Point")
        .unwrap_or_else(|| panic!("struct Point missing: {syms:?}"));
    // classify_kind: contains("struct") → Struct
    assert_eq!(
        point.kind,
        SymbolKind::Struct,
        "struct must classify as SymbolKind::Struct"
    );
}

#[test]
fn interface_declaration_emits_symbol_with_interface_kind() {
    let src = b"public interface IPrintable { void Print(); }\n";
    let syms = parse_source(Lang::CSharp, src).expect("parse_source Ok");
    let iface = syms
        .iter()
        .find(|s| s.name == "IPrintable")
        .unwrap_or_else(|| panic!("interface IPrintable missing: {syms:?}"));
    assert_eq!(
        iface.kind,
        SymbolKind::Interface,
        "interface must classify as SymbolKind::Interface"
    );
}
