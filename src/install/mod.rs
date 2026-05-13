//! Installer subsystem. Two orthogonal install paths:
//!
//! - [`mcp_config`] writes the MCP server registration so LLM clients can
//!   launch `graphatlas mcp` (S-002 v1.0 install).
//! - [`hook`] writes the PostToolUse hook so agents auto-call
//!   `ga_reindex` after Edit/Write/Bash (v1.5 PR7 triggers spec).
//!
//! Re-exports the public API used by `cmd_install` in `main.rs` so
//! external callers don't care about the internal module split.

pub mod claudemd;
pub mod hook;
pub mod json_io;
pub mod mcp_config;
pub mod permissions;
pub mod session_hook;
pub mod skill;

// Re-exports preserving the pre-split public API surface.
pub use hook::{
    install_hook, install_hook_at, uninstall_hook, uninstall_hook_at, verify_hook, verify_hook_at,
    HookClient, HookOutcome, VerifyOutcome,
};
pub use mcp_config::{write_mcp_config, Client, InstallOutcome};
