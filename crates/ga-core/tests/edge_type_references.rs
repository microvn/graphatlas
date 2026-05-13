//! Foundation-C15 — `EdgeType::References` variant for value-reference edges.

use ga_core::EdgeType;

#[test]
fn references_variant_exists() {
    let _ = EdgeType::References;
}

#[test]
fn references_serializes_as_screaming_snake_case() {
    // Per existing `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]` on EdgeType.
    let json = serde_json::to_string(&EdgeType::References).unwrap();
    assert_eq!(json, "\"REFERENCES\"");
}

#[test]
fn references_round_trips_through_json() {
    let original = EdgeType::References;
    let json = serde_json::to_string(&original).unwrap();
    let back: EdgeType = serde_json::from_str(&json).unwrap();
    assert_eq!(back, original);
}
