//! Claude Code instruction surface — stub.
//!
//! Claude Code has 3-4 instruction components (skill, CLAUDE.md block,
//! settings.json permissions, optional SessionStart hook) instead of
//! the single instruction file used by other platforms. cmd_init
//! handles the breakdown directly. This module exists for symmetry
//! with the other platform modules so the [`super::Platform`] enum
//! dispatch table is uniform.

// Intentionally empty — install/uninstall/preflight live in
// cmd_init.rs because they compose multiple installers from
// install/{skill, claudemd, permissions, session_hook}.rs.
