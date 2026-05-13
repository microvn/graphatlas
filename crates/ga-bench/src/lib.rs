//! Benchmarks S-001 — per-UC bench harness. Ground-truth loader, retriever
//! adapters (ga + baselines), scorers, and Markdown leaderboard writer.

pub mod error;
pub mod fixture_workspace;
pub mod git_pin;
pub mod gt_gen;
pub mod leaderboard;
pub mod m1_ground_truth;
pub mod m2_ground_truth;
pub mod m2_markdown;
pub mod m2_runner;
pub mod m3_architecture;
pub mod m3_dead_code;
pub mod m3_hubs;
pub mod m3_minimal_context;
pub mod m3_rename_safety;
pub mod m3_risk;
pub mod m3_runner;
pub mod m3_score;
pub mod manifest;
pub mod mcp;
pub mod retriever;
pub mod retrievers;
pub mod runner;
pub mod score;
pub mod signals;

pub use retriever::{ImpactActual, Retriever};

pub use error::BenchError;
pub use leaderboard::{write_leaderboard, LeaderEntry, Leaderboard};
pub use m1_ground_truth::{M1GroundTruth, M1Task, EXPECTED_SCHEMA_VERSION};
pub use m2_ground_truth::{M2GroundTruth, M2Task, Split};
pub use m2_runner::{M2Report, RepoGroup, RetrieverEntry, RunOpts, TaskScore};
