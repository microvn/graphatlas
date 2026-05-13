//! Platform abstraction for `ga init` — one enum variant per supported
//! LLM-coding-agent. Each platform owns its own instruction-file
//! installer (`install/uninstall/preflight`). The MCP server
//! registration is handled separately by [`super::mcp_config`].
//!
//! Platforms supported in v1 (this commit):
//! - Claude Code      (existing skill + CLAUDE.md + permissions + hook)
//! - Cursor           (.cursor/rules/graphatlas.mdc)
//! - Cline            (.clinerules, single-file form)
//! - Codex CLI        (AGENTS.md project-root managed block)
//! - Gemini CLI       (GEMINI.md project-root managed block)
//!
//! Detection heuristic: probe the user-home config dir for the agent.
//! Heuristics are intentionally loose — false negatives surface as
//! "not detected" but the user can still install via positional CLI
//! arg or interactive override.

pub mod claude_code;
pub mod cline_rules;
pub mod codex_agents;
pub mod cursor_rules;
pub mod gemini_md;
pub mod windsurf_rules;

use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    ClaudeCode,
    Cursor,
    Cline,
    CodexCli,
    GeminiCli,
    Windsurf,
    Continue,
    Zed,
}

impl Platform {
    pub const ALL: &'static [Platform] = &[
        Self::ClaudeCode,
        Self::Cursor,
        Self::Cline,
        Self::CodexCli,
        Self::GeminiCli,
        Self::Windsurf,
        Self::Continue,
        Self::Zed,
    ];

    /// Canonical kebab-case slug used in CLI positional args + flags.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Cline => "cline",
            Self::CodexCli => "codex",
            Self::GeminiCli => "gemini",
            Self::Windsurf => "windsurf",
            Self::Continue => "continue",
            Self::Zed => "zed",
        }
    }

    /// Human-readable name shown in interactive picker.
    pub fn display(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Cline => "Cline",
            Self::CodexCli => "Codex CLI",
            Self::GeminiCli => "Gemini CLI",
            Self::Windsurf => "Windsurf",
            Self::Continue => "Continue",
            Self::Zed => "Zed",
        }
    }

    /// PostToolUse reindex hook client this platform maps to, if any.
    /// `ga init` uses this to auto-install the reindex hook alongside
    /// MCP + instruction surface. Platforms without native post-tool
    /// hooks (Cline, Continue, Gemini, Windsurf, Zed) return None and
    /// rely on the agent to invoke `ga_reindex` itself when stale.
    pub fn hook_client(&self) -> Option<super::hook::HookClient> {
        match self {
            Self::ClaudeCode => Some(super::hook::HookClient::ClaudeCode),
            Self::Cursor => Some(super::hook::HookClient::Cursor),
            Self::CodexCli => Some(super::hook::HookClient::Codex),
            Self::Cline => Some(super::hook::HookClient::ClineScript),
            Self::GeminiCli => Some(super::hook::HookClient::GeminiCli),
            Self::Windsurf => Some(super::hook::HookClient::Windsurf),
            // Continue + Zed — no stable hook system to integrate.
            Self::Continue | Self::Zed => None,
        }
    }

    /// Parse a user-supplied slug. Accepts a few common aliases.
    pub fn from_slug(s: &str) -> Option<Platform> {
        match s.to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "claudecode" => Some(Self::ClaudeCode),
            "cursor" => Some(Self::Cursor),
            "cline" => Some(Self::Cline),
            "codex" | "codex-cli" => Some(Self::CodexCli),
            "gemini" | "gemini-cli" => Some(Self::GeminiCli),
            "windsurf" => Some(Self::Windsurf),
            "continue" | "continue-dev" => Some(Self::Continue),
            "zed" => Some(Self::Zed),
            _ => None,
        }
    }

    /// Detect whether the user has this agent installed by probing for
    /// its config dir under $HOME. Loose heuristic — directory presence
    /// is a strong-enough signal because every agent's first-run
    /// creates its config dir.
    pub fn detect(&self) -> bool {
        let Some(home) = home_dir() else {
            return false;
        };
        match self {
            Self::ClaudeCode => home.join(".claude").exists(),
            Self::Cursor => home.join(".cursor").exists(),
            Self::Cline => {
                // Cline runs as a VS Code extension; the extension's
                // globalStorage dir is per-OS. Probe the most-likely
                // paths and return true on any match.
                cline_global_storage_dir(&home).is_some()
            }
            Self::CodexCli => home.join(".codex").exists(),
            Self::GeminiCli => home.join(".gemini").exists(),
            Self::Windsurf => home.join(".codeium/windsurf").exists(),
            Self::Continue => home.join(".continue").exists(),
            Self::Zed => zed_settings_dir(&home).exists(),
        }
    }

    /// Install the platform's *instruction* surface in the given
    /// project root. The MCP server registration is handled separately
    /// by [`super::mcp_config`].
    pub fn install_instructions(&self, project_root: &Path) -> Result<InstructionOutcome> {
        match self {
            // Claude Code's instruction surface (skill + CLAUDE.md +
            // permissions + optional hook) is wired by cmd_init
            // directly because it has 3-4 sub-components instead of
            // one. claude_code::install() returns the noop sentinel
            // here; cmd_init handles the breakdown.
            Self::ClaudeCode => Ok(InstructionOutcome::DelegatedToClaudeCodeFlow),
            Self::Cursor => cursor_rules::install(project_root).map(InstructionOutcome::Cursor),
            Self::Cline => cline_rules::install(project_root).map(InstructionOutcome::Cline),
            Self::CodexCli => codex_agents::install(project_root).map(InstructionOutcome::Codex),
            Self::GeminiCli => gemini_md::install(project_root).map(InstructionOutcome::Gemini),
            Self::Windsurf => {
                windsurf_rules::install(project_root).map(InstructionOutcome::Windsurf)
            }
            // Continue + Zed: MCP-only, no project rules format → no-op.
            Self::Continue | Self::Zed => Ok(InstructionOutcome::McpOnly),
        }
    }

    /// Uninstall the instruction file (managed block only).
    pub fn uninstall_instructions(&self, project_root: &Path) -> Result<bool> {
        match self {
            Self::ClaudeCode => Ok(false),
            Self::Cursor => cursor_rules::uninstall(project_root),
            Self::Cline => cline_rules::uninstall(project_root),
            Self::CodexCli => codex_agents::uninstall(project_root),
            Self::GeminiCli => gemini_md::uninstall(project_root),
            Self::Windsurf => windsurf_rules::uninstall(project_root),
            Self::Continue | Self::Zed => Ok(false),
        }
    }

    /// Pre-flight one-line summary of what `install_instructions`
    /// would do. Returns the target path and the action verb.
    pub fn preflight(&self, project_root: &Path) -> PreflightSummary {
        match self {
            Self::ClaudeCode => PreflightSummary {
                target: project_root.join(".claude/skills/graphatlas.md"),
                state: "skill + CLAUDE.md + permissions",
            },
            Self::Cursor => cursor_rules::preflight(project_root),
            Self::Cline => cline_rules::preflight(project_root),
            Self::CodexCli => codex_agents::preflight(project_root),
            Self::GeminiCli => gemini_md::preflight(project_root),
            Self::Windsurf => windsurf_rules::preflight(project_root),
            Self::Continue => PreflightSummary {
                target: project_root.join(".continue/mcpServers/graphatlas.json"),
                state: "MCP-only — no project rules format",
            },
            Self::Zed => PreflightSummary {
                target: project_root.join(".zed/settings.json"),
                state: "MCP-only — no project rules format",
            },
        }
    }
}

#[derive(Debug)]
pub enum InstructionOutcome {
    DelegatedToClaudeCodeFlow,
    /// Platform has no instruction surface (Zed, Continue) — MCP-only.
    McpOnly,
    Cursor(SimpleOutcome),
    Cline(SimpleOutcome),
    Codex(SimpleOutcome),
    Gemini(SimpleOutcome),
    Windsurf(SimpleOutcome),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SimpleOutcome {
    Created(PathBuf),
    Updated(PathBuf),
    Unchanged(PathBuf),
}

#[derive(Debug, Clone)]
pub struct PreflightSummary {
    pub target: PathBuf,
    pub state: &'static str,
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Return Cline's per-OS globalStorage dir if the parent VS Code-like
/// editor data dir exists. Probes Code, Code - Insiders, Cursor,
/// Windsurf in that order — VS Code first because the saoudrizwan
/// extension targets it primarily.
pub fn cline_global_storage_dir(home: &Path) -> Option<PathBuf> {
    let candidates = cline_globalstorage_candidates(home);
    candidates.into_iter().find(|p| p.exists())
}

fn cline_globalstorage_candidates(home: &Path) -> Vec<PathBuf> {
    let editors = ["Code", "Code - Insiders", "Cursor", "Windsurf"];
    let mut out = Vec::with_capacity(editors.len());
    for editor in editors {
        out.push(per_os_globalstorage(home, editor));
    }
    out
}

/// Zed's user-global settings dir per-OS.
/// - macOS:   ~/Library/Application Support/Zed
/// - Linux:   ~/.config/zed
/// - Windows: %APPDATA%\Zed
pub fn zed_settings_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Zed")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("Zed")
    } else {
        home.join(".config/zed")
    }
}

/// Cline's MCP settings file path for a given $HOME. Defaults to the
/// VS Code editor's globalStorage location for the saoudrizwan.claude-dev
/// extension; per-OS variants follow VS Code's docs.
pub fn per_os_cline_settings(home: &Path) -> PathBuf {
    per_os_globalstorage(home, "Code").join("cline_mcp_settings.json")
}

fn per_os_globalstorage(home: &Path, editor: &str) -> PathBuf {
    // macOS: ~/Library/Application Support/<editor>/User/globalStorage/saoudrizwan.claude-dev/settings/
    // Linux: ~/.config/<editor>/User/globalStorage/saoudrizwan.claude-dev/settings/
    // Windows: %APPDATA%\<editor>\User\globalStorage\saoudrizwan.claude-dev\settings\
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        home.join(".config")
    };
    base.join(editor)
        .join("User/globalStorage/saoudrizwan.claude-dev/settings")
}
