use ga_mcp::{MCP_PROTOCOL_VERSION, SERVER_NAME};

#[test]
fn protocol_version_is_full_date() {
    // Per R37: must be full YYYY-MM-DD, not YYYY-MM.
    assert_eq!(MCP_PROTOCOL_VERSION, "2025-11-25");
    assert_eq!(MCP_PROTOCOL_VERSION.len(), 10);
    assert!(MCP_PROTOCOL_VERSION.chars().filter(|&c| c == '-').count() == 2);
}

#[test]
fn server_name_matches_binary() {
    assert_eq!(SERVER_NAME, "graphatlas");
}

// run_stdio_stub_errors removed per infra:S-003 reframe (2026-04-24) —
// run_stdio is no longer a stub; integration coverage lives in
// tests/stdio_integration.rs (AS-008/009/010).
