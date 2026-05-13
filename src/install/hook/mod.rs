//! v1.5 PR7 — Hook installer (triggers spec S-001 + S-002).
//!
//! Separate from `mcp_config::write_mcp_config`: the MCP-config writer
//! wires the *server* registration so the agent can talk to graphatlas
//! at all, while the hook installer wires *post-edit triggers* so the
//! agent auto-calls `ga_reindex` after Edit/Write/Bash. They target
//! different files, ship orthogonally, and either can be installed
//! without the other.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

mod backends;
use backends::{
    ensure_parent_dir, install_json_hook, install_toml_hook, refuse_symlink_unless,
    uninstall_json_hook, uninstall_toml_hook, verify_json_hook, verify_toml_hook,
};

/// Matcher pattern the hook fires on. AS-001 spec literal.
pub(super) const HOOK_MATCHER: &str = "Edit|Write|Bash";
/// MCP tool the hook dispatches. AS-001 spec literal.
pub(super) const HOOK_TOOL: &str = "ga_reindex";
/// MCP server name (matches `mcpServers.graphatlas` from `write_mcp_config`).
pub(super) const HOOK_SERVER: &str = "graphatlas";
/// Hook type literal. Claude Code 2026-05 hooks engine accepts five
/// types — `command`, `http`, `mcp_tool`, `prompt`, `agent` — per
/// https://code.claude.com/docs/en/hooks. The prior literal `"mcp"`
/// (PR7 spec drift) was rejected with "type: Invalid input" and
/// caused Claude Code to skip the entire `settings.json`. Centralizing
/// the literal here so future drift is caught by `grep HOOK_TYPE`
/// rather than by a user opening Claude Code and seeing the error
/// panel.
pub(super) const HOOK_TYPE: &str = "mcp_tool";

/// Agents this installer can write hook config for. PR7 ships
/// claude-code (P0, S-001) + cursor/codex (P1, S-002). Aider/Antigravity
/// deferred — no stable hook API in 2026-05.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookClient {
    ClaudeCode,
    Cursor,
    Codex,
}

impl std::str::FromStr for HookClient {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude-code" | "claude_code" | "claudecode" => Ok(Self::ClaudeCode),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            other => Err(anyhow!(
                "unknown hook client `{other}` — supported: claude-code, cursor, codex"
            )),
        }
    }
}

impl HookClient {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
        }
    }

    /// Resolve the hook config path for `client`. claude-code + cursor
    /// are project-local (so per-repo opt-in works); codex is HOME-global
    /// per its config convention.
    pub fn config_path(self, project_root: &Path) -> Result<PathBuf> {
        match self {
            Self::ClaudeCode => Ok(project_root.join(".claude").join("settings.json")),
            Self::Cursor => Ok(project_root.join(".cursor").join("mcp.json")),
            Self::Codex => {
                let home = std::env::var("HOME").map_err(|_| {
                    anyhow!("HOME env var not set; cannot resolve codex config path")
                })?;
                Ok(PathBuf::from(home).join(".codex").join("config.toml"))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum HookOutcome {
    Created {
        client: HookClient,
        path: PathBuf,
    },
    /// File existed and contained no GA entry — appended.
    Added {
        client: HookClient,
        path: PathBuf,
    },
    /// File existed and already contained the GA entry — no-op.
    AlreadyPresent {
        client: HookClient,
        path: PathBuf,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Hook entry is present and matches the expected matcher + tool.
    Ok,
    /// File does not exist or does not contain the hook entry.
    Missing { hint: String },
    /// File exists but the entry is malformed or partial.
    Malformed { hint: String },
}

/// AS-001 + AS-001b + AS-002 — install (or no-op merge) the PostToolUse
/// hook entry for `client` under `project_root`. Atomic, symlink-defended,
/// idempotent.
pub fn install_hook(
    client: HookClient,
    project_root: &Path,
    follow_symlinks: bool,
) -> Result<HookOutcome> {
    let path = client.config_path(project_root)?;
    install_hook_at(client, &path, follow_symlinks)
}

/// Lower-level entry point: install at an explicit path, bypassing the
/// `HookClient::config_path` resolution. Test fixtures use this to point
/// Codex at a TempDir-rooted config without setting `$HOME` globally.
pub fn install_hook_at(
    client: HookClient,
    path: &Path,
    follow_symlinks: bool,
) -> Result<HookOutcome> {
    ensure_parent_dir(path)?;
    refuse_symlink_unless(path, follow_symlinks)?;
    match client {
        HookClient::ClaudeCode | HookClient::Cursor => install_json_hook(client, path),
        HookClient::Codex => install_toml_hook(client, path),
    }
}

/// AS-004 — remove the GA hook entry. If `PostToolUse` becomes empty
/// after removal, the array is dropped (other top-level keys preserved).
/// File is kept (not deleted) so unrelated config survives.
pub fn uninstall_hook(
    client: HookClient,
    project_root: &Path,
    follow_symlinks: bool,
) -> Result<()> {
    let path = client.config_path(project_root)?;
    uninstall_hook_at(client, &path, follow_symlinks)
}

pub fn uninstall_hook_at(client: HookClient, path: &Path, follow_symlinks: bool) -> Result<()> {
    refuse_symlink_unless(path, follow_symlinks)?;
    if !path.exists() {
        return Ok(());
    }
    match client {
        HookClient::ClaudeCode | HookClient::Cursor => uninstall_json_hook(path),
        HookClient::Codex => uninstall_toml_hook(path),
    }
}

/// AS-003 — report whether the hook is present + matches the expected
/// matcher + tool. Distinguishes Missing vs Malformed so callers can
/// emit actionable diagnostics.
pub fn verify_hook(client: HookClient, project_root: &Path) -> Result<VerifyOutcome> {
    let path = client.config_path(project_root)?;
    verify_hook_at(client, &path)
}

pub fn verify_hook_at(client: HookClient, path: &Path) -> Result<VerifyOutcome> {
    if !path.exists() {
        return Ok(VerifyOutcome::Missing {
            hint: format!(
                "config not found at {}; run `graphatlas install --hook {}`",
                path.display(),
                client.as_str()
            ),
        });
    }
    match client {
        HookClient::ClaudeCode | HookClient::Cursor => verify_json_hook(path),
        HookClient::Codex => verify_toml_hook(path),
    }
}
