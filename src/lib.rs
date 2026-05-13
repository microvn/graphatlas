//! graphatlas crate library — re-exports the subcommand modules so integration
//! tests can drive them without spawning subprocesses. Binary (`src/main.rs`)
//! uses these same modules via the clap dispatcher.

pub mod bench_cmd;
pub mod cmd_hook;
pub mod cmd_init;
pub mod doctor;
pub mod install;
pub mod mcp_cmd;
