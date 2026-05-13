use ga_core::{Edge, EdgeType, File, IndexState, Lang, Symbol, SymbolKind};

#[test]
fn file_serde_round_trip() {
    let f = File {
        path: "src/lib.rs".into(),
        lang: Lang::Rust,
        mtime_ns: 1_700_000_000_000_000_000,
        size: 4096,
        hash: [0u8; 32],
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: File = serde_json::from_str(&json).unwrap();
    assert_eq!(back.path, "src/lib.rs");
    assert_eq!(back.lang, Lang::Rust);
    assert_eq!(back.size, 4096);
}

#[test]
fn symbol_serde_round_trip() {
    let s = Symbol {
        id: "a.py:10:foo".into(),
        name: "foo".into(),
        kind: SymbolKind::Function,
        file: "a.py".into(),
        line: 10,
        line_end: 10,
        enclosing: None,
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"kind\":\"function\""));
    let back: Symbol = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "foo");
}

#[test]
fn edge_serde_screaming_snake() {
    let e = Edge {
        edge_type: EdgeType::TestedBy,
        from: "x".into(),
        to: "y".into(),
        confidence: 0.9,
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains("\"type\":\"TESTED_BY\""));
}

#[test]
fn index_state_serde_lowercase() {
    assert_eq!(
        serde_json::to_string(&IndexState::Building).unwrap(),
        "\"building\""
    );
    assert_eq!(
        serde_json::to_string(&IndexState::Complete).unwrap(),
        "\"complete\""
    );
}
