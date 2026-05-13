//! `ga hook <subcommand>` — hidden CLI surface invoked by Claude Code
//! hooks installed via `ga init --with-hook`.
//!
//! Subcommands:
//! - `session-start`: print the discovery protocol reminder to stdout.
//!   Claude Code SessionStart hooks emit hook output verbatim into the
//!   session's system context, so the reminder lands on every session
//!   start (one inject per session, not per prompt).

use anyhow::Result;

/// Reminder text baked into the binary. Kept at the assets/ layer so
/// the same content is reusable for non-Claude agents in v2.
pub const SESSION_START_REMINDER: &str = include_str!("../assets/session-start-reminder.txt");

pub fn cmd_hook_session_start() -> Result<()> {
    // Write to stdout exactly as-is; trailing newline already present in
    // the asset file. Claude Code captures the full stdout block.
    print!("{SESSION_START_REMINDER}");
    Ok(())
}
