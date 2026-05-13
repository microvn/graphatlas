//! Bench error types. Distinct from ga_core::Error so bench failures don't
//! pollute MCP JSON-RPC code space.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("fixture missing at {path}. Run `git submodule update --init --recursive`.")]
    FixtureMissing { path: String },

    #[error("ground-truth schema_version mismatch: got {got}, expected {expected}. Run `scripts/refresh-gt.sh` to regenerate.")]
    SchemaMismatch { got: u32, expected: u32 },

    #[error("ground-truth JSON malformed at {path}: {reason}")]
    GroundTruthMalformed { path: String, reason: String },

    #[error(
        "unknown UC `{0}` — supported: callers, callees, importers, symbols, file_summary, impact"
    )]
    UnknownUc(String),

    #[error(
        "unknown M3 UC `{0}`; valid: dead_code|rename_safety|minimal_context|architecture|risk"
    )]
    UnknownM3Uc(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("graphatlas query error: {0}")]
    Query(String),
}

impl From<ga_core::Error> for BenchError {
    fn from(e: ga_core::Error) -> Self {
        BenchError::Query(e.to_string())
    }
}
