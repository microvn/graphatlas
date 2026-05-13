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
mod extra_backends;
use backends::{
    ensure_parent_dir, install_json_hook, refuse_symlink_unless, uninstall_json_hook,
    verify_json_hook,
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
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookClient {
    /// Claude Code — JSON `.claude/settings.json` `hooks.PostToolUse`,
    /// entry `type: "mcp_tool"`.
    ClaudeCode,
    /// Cursor — same JSON shape as Claude in `.cursor/mcp.json`.
    Cursor,
    /// Codex CLI — TOML `~/.codex/config.toml` `[[hooks.PostToolUse]]`,
    /// `type = "mcp_tool"`.
    Codex,
    /// Cline VS Code extension — executable script file at
    /// `.clinerules/hooks/PostToolUse`. Reads JSON stdin, calls
    /// `<binary> reindex`, writes `{"cancel":false}` to stdout.
    ClineScript,
    /// Gemini CLI — JSON `.gemini/settings.json` `hooks.AfterTool`,
    /// entry `type: "command"` (shell). Matcher = tool name regex.
    GeminiCli,
    /// Windsurf — JSON `.windsurf/hooks.json` `hooks.post_write_code`
    /// + `hooks.post_run_command`. Entry shape `{command, powershell}`.
    Windsurf,
}

impl std::str::FromStr for HookClient {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude-code" | "claude_code" | "claudecode" => Ok(Self::ClaudeCode),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            "cline" | "cline-script" => Ok(Self::ClineScript),
            "gemini" | "gemini-cli" => Ok(Self::GeminiCli),
            "windsurf" => Ok(Self::Windsurf),
            other => Err(anyhow!(
                "unknown hook client `{other}` — supported: \
                 claude-code, cursor, codex, cline, gemini, windsurf"
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
            Self::ClineScript => "cline",
            Self::GeminiCli => "gemini",
            Self::Windsurf => "windsurf",
        }
    }

    /// Whether this client expresses hooks via `type: "mcp_tool"` (the
    /// Claude Code spec literal). The other clients use shell commands
    /// invoking `<binary> reindex` instead.
    pub fn uses_mcp_tool_hook(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Cursor | Self::Codex)
    }

    /// Resolve the hook config path for `client`. Project-local where
    /// supported (claude-code, cursor, gemini, windsurf workspace,
    /// cline); HOME-global for codex.
    ///
    /// Note: Cursor hooks live in `.cursor/hooks.json` (NOT
    /// `.cursor/mcp.json` — that's MCP server registry). Spec verified
    /// 2026-05 against cursor.com/docs/agent/hooks.
    pub fn config_path(self, project_root: &Path) -> Result<PathBuf> {
        match self {
            Self::ClaudeCode => Ok(project_root.join(".claude").join("settings.json")),
            Self::Cursor => Ok(project_root.join(".cursor").join("hooks.json")),
            Self::Codex => {
                let home = std::env::var("HOME").map_err(|_| {
                    anyhow!("HOME env var not set; cannot resolve codex config path")
                })?;
                Ok(PathBuf::from(home).join(".codex").join("config.toml"))
            }
            Self::ClineScript => Ok(project_root
                .join(".clinerules")
                .join("hooks")
                .join("PostToolUse")),
            Self::GeminiCli => Ok(project_root.join(".gemini").join("settings.json")),
            Self::Windsurf => Ok(project_root.join(".windsurf").join("hooks.json")),
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
    let binary = std::env::current_exe()?;
    install_hook_at_with_binary(client, path, &binary, follow_symlinks)
}

/// Like [`install_hook_at`] but with an explicit binary path embedded in
/// shell-command hooks (Cline / Gemini / Windsurf). Tests use this to
/// inject a deterministic path. Ignored for mcp_tool-style hooks.
pub fn install_hook_at_with_binary(
    client: HookClient,
    path: &Path,
    binary_path: &Path,
    follow_symlinks: bool,
) -> Result<HookOutcome> {
    ensure_parent_dir(path)?;
    refuse_symlink_unless(path, follow_symlinks)?;
    // TOCTOU defense: hold advisory flock for the read→modify→write
    // window so concurrent installers (permissions, session_hook,
    // reindex hook all touch .claude/settings.json) serialize cleanly
    // across processes.
    let _lock = crate::install::json_io::lock_file(path)?;
    match client {
        // Claude Code uses `type: "mcp_tool"` — calls MCP server directly.
        HookClient::ClaudeCode => install_json_hook(client, path),
        // Cursor + Codex shape-verified 2026-05: both use shell-command
        // hooks (type:"command"), NOT type:"mcp_tool".
        HookClient::Cursor => {
            extra_backends::install_cursor_hooks(client, path, binary_path)
        }
        HookClient::Codex => extra_backends::install_codex_toml_hook(client, path, binary_path),
        HookClient::ClineScript => {
            extra_backends::install_cline_script(client, path, binary_path)
        }
        HookClient::GeminiCli => {
            extra_backends::install_gemini_aftertool(client, path, binary_path)
        }
        HookClient::Windsurf => {
            extra_backends::install_windsurf_postwrite(client, path, binary_path)
        }
    }
}

/// Convenience that resolves the default config path + uses
/// `current_exe()` as binary path. Equivalent to [`install_hook`] but
/// accepts an explicit binary for shell-command hooks.
pub fn install_hook_with_binary(
    client: HookClient,
    project_root: &Path,
    binary_path: &Path,
    follow_symlinks: bool,
) -> Result<HookOutcome> {
    let path = client.config_path(project_root)?;
    install_hook_at_with_binary(client, &path, binary_path, follow_symlinks)
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
        HookClient::ClaudeCode => uninstall_json_hook(path),
        HookClient::Cursor => extra_backends::uninstall_cursor_hooks(path),
        HookClient::Codex => extra_backends::uninstall_codex_toml_hook(path),
        HookClient::ClineScript => extra_backends::uninstall_cline_script(path),
        HookClient::GeminiCli => extra_backends::uninstall_gemini_aftertool(path),
        HookClient::Windsurf => extra_backends::uninstall_windsurf_postwrite(path),
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
        HookClient::ClaudeCode => verify_json_hook(path),
        HookClient::Cursor => extra_backends::verify_cursor_hooks(path),
        HookClient::Codex => extra_backends::verify_codex_toml_hook(path),
        HookClient::ClineScript => extra_backends::verify_cline_script(path),
        HookClient::GeminiCli => extra_backends::verify_gemini_aftertool(path),
        HookClient::Windsurf => extra_backends::verify_windsurf_postwrite(path),
    }
}
