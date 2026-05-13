//! M3 gate smoke tests — S-001 cycle 1 (AS-001 + AS-002 core contract).
//!
//! Per spec:
//! - AS-001: m3_runner exposes `pub fn run(M3GateConfig) -> Result<Vec<M3LeaderboardRow>, BenchError>`
//!           sync, mirror m2_runner style.
//! - AS-002: unknown UC name → clear error citing valid UCs (incl. `risk`,
//!   pulled out of Phase 3 deferral once GitLogMiner anchor + Hr-text rule shipped).

use ga_bench::m3_runner::{run, M3GateConfig, M3LeaderboardRow, SpecStatus};
use ga_bench::BenchError;

/// AS-001.T1 — `m3_runner::run` exposes the agreed public contract:
/// takes ownership of `M3GateConfig`, returns `Result<Vec<M3LeaderboardRow>, BenchError>`.
/// On a config the runner can't actually score yet (empty retriever set), it must
/// return an `Ok` with empty rows rather than panic — proves the type signature
/// compiles and the surface accepts a valid config.
#[test]
fn as_001_run_signature_accepts_valid_config_returns_empty_when_no_retrievers() {
    let cfg = M3GateConfig {
        uc: "minimal_context".to_string(),
        fixture: "preact".to_string(),
        retrievers: vec![],
        gate: "m3".to_string(),
    };
    let result: Result<Vec<M3LeaderboardRow>, BenchError> = run(cfg);
    let rows = result.expect("valid config with no retrievers should return Ok");
    assert!(
        rows.is_empty(),
        "expected empty Vec when retriever list is empty, got {} rows",
        rows.len()
    );
}

/// AS-002.T1 — unknown UC name returns a clear `BenchError::UnknownUc`-style
/// error whose message lists all five valid M3 UCs.
#[test]
fn as_002_unknown_uc_returns_clear_error_listing_all_five_ucs() {
    let cfg = M3GateConfig {
        uc: "bogus_uc".to_string(),
        fixture: "preact".to_string(),
        retrievers: vec!["ga".to_string()],
        gate: "m3".to_string(),
    };
    let err = run(cfg).expect_err("unknown UC must error");
    let msg = err.to_string();
    assert!(
        msg.contains("bogus_uc"),
        "error message must echo offending UC name; got: {msg}"
    );
    for uc in [
        "dead_code",
        "rename_safety",
        "minimal_context",
        "architecture",
        "risk",
    ] {
        assert!(
            msg.contains(uc),
            "error message must list valid M3 UC `{uc}`; got: {msg}"
        );
    }
}

/// AS-001.T2 (partial) — `M3LeaderboardRow.spec_status` field exists with the four
/// declared variants (PASS / FAIL / TAUTOLOGICAL / DEFERRED). Exercises the type
/// surface; runtime production of these statuses is verified in AS-004 cycle.
#[test]
fn as_004_spec_status_enum_has_four_variants() {
    let _ = SpecStatus::Pass;
    let _ = SpecStatus::Fail;
    let _ = SpecStatus::Tautological;
    let _ = SpecStatus::Deferred;
    // If the enum is missing any variant, this file fails to compile — that's the
    // assertion. Runtime check is trivially "compiled = passed".
}
