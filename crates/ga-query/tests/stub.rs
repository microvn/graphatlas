use ga_query::{CallKind, CallerEntry};

#[test]
fn caller_entry_serde_round_trip() {
    let c = CallerEntry {
        file: "a.py".into(),
        symbol: "bar".into(),
        line: 10,
        call_site_line: 15,
        confidence: 1.0,
        kind: CallKind::Call,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: CallerEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.symbol, "bar");
    assert_eq!(back.call_site_line, 15);
    assert_eq!(back.kind, CallKind::Call);
}
