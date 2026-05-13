//! `graphatlas install --client <name>` — wire the MCP server entry for
//! an LLM coding agent so the user doesn't have to hand-edit config.
//!
//! Behaviour:
//!   1. Resolve default config path per client (handles per-OS / TOML / JSON).
//!   2. If config exists: parse → deep-merge → write back, preserving keys.
//!   3. Write `.bak` of original before touching the file.
//!   4. Atomic write: tmp → rename.
//!   5. If config was missing: create fresh `{mcpServers: {graphatlas: …}}`
//!      (JSON) or `[mcp_servers.graphatlas]` (TOML for Codex CLI).
//!
//! Per-client config paths verified against vendor docs 2026-05:
//! - Claude Code:  ~/.claude/mcp.json (JSON, `mcpServers.X`)
//! - Cursor:       ~/.cursor/mcp.json (JSON, `mcpServers.X`)
//! - Cline:        per-OS VS Code extension globalStorage (JSON)
//! - Codex CLI:    ~/.codex/config.toml (TOML, `[mcp_servers.X]`)
//! - Gemini CLI:   ~/.gemini/settings.json (JSON, `mcpServers.X`)

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::json_io::{atomic_write_bytes, read_toml_or_empty, refuse_symlink_unless};

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Claude,
    Cursor,
    Cline,
    Codex,
    Gemini,
    Windsurf,
    Continue,
    Zed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
}

impl std::str::FromStr for Client {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Ok(Self::Claude),
            "cursor" => Ok(Self::Cursor),
            "cline" => Ok(Self::Cline),
            "codex" | "codex-cli" => Ok(Self::Codex),
            "gemini" | "gemini-cli" => Ok(Self::Gemini),
            "windsurf" => Ok(Self::Windsurf),
            "continue" | "continue-dev" => Ok(Self::Continue),
            "zed" => Ok(Self::Zed),
            other => Err(anyhow!(
                "unknown client `{other}` — supported: claude, cursor, cline, codex, gemini, windsurf, continue, zed"
            )),
        }
    }
}

impl Client {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Cline => "cline",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Windsurf => "windsurf",
            Self::Continue => "continue",
            Self::Zed => "zed",
        }
    }

    pub fn format(self) -> Format {
        match self {
            Self::Codex => Format::Toml,
            _ => Format::Json,
        }
    }

    /// JSON root key the server entry lives under. Most clients use
    /// `mcpServers`; Zed is the outlier with `context_servers`.
    pub fn json_root_key(self) -> &'static str {
        match self {
            Self::Zed => "context_servers",
            _ => "mcpServers",
        }
    }

    /// Resolve the default user-global config path for this client.
    pub fn default_config_path_for_home(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude/mcp.json"),
            Self::Cursor => home.join(".cursor/mcp.json"),
            Self::Cline => super::platforms::per_os_cline_settings(home),
            Self::Codex => home.join(".codex/config.toml"),
            Self::Gemini => home.join(".gemini/settings.json"),
            Self::Windsurf => home.join(".codeium/windsurf/mcp_config.json"),
            Self::Continue => home.join(".continue/mcpServers/graphatlas.json"),
            Self::Zed => super::platforms::zed_settings_dir(home).join("settings.json"),
        }
    }

    /// Project-local config path, if the platform supports it.
    pub fn project_config_path(self, project_root: &Path) -> Option<PathBuf> {
        match self {
            Self::Claude => None,
            Self::Cursor => Some(project_root.join(".cursor/mcp.json")),
            Self::Cline => None,
            Self::Codex => Some(project_root.join(".codex/config.toml")),
            Self::Gemini => Some(project_root.join(".gemini/settings.json")),
            // Windsurf has no project-local MCP config — falls back to user-global.
            Self::Windsurf => None,
            // Continue's MCP servers are one-file-per-server in
            // `.continue/mcpServers/<name>.json` at project root.
            Self::Continue => Some(project_root.join(".continue/mcpServers/graphatlas.json")),
            // Zed supports project-local `.zed/settings.json`.
            Self::Zed => Some(project_root.join(".zed/settings.json")),
        }
    }
}

#[derive(Debug)]
pub enum InstallOutcome {
    Created {
        client: Client,
        config_path: PathBuf,
    },
    Updated {
        client: Client,
        config_path: PathBuf,
        had_existing_entry: bool,
    },
}

/// Write the `graphatlas` MCP entry into the config for `client`.
///
/// - `config_path`: explicit override; when `None`, resolve from `$HOME`
///   + per-client default.
/// - `binary_path`: command to launch the MCP server. Use
///   `std::env::current_exe()` in production.
pub fn write_mcp_config(
    client: Client,
    config_path: Option<&Path>,
    binary_path: &Path,
) -> Result<InstallOutcome> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => default_path_for(client)?,
    };

    // Defense-in-depth: refuse to write through a symlink at the
    // target. Mirrors install/hook/mod.rs::install_hook_at_with_binary
    // which has done this since v1.5 PR7. Earlier mcp_config skipped
    // this check — an attacker-controlled `~/.cursor/mcp.json` symlink
    // could redirect writes to sensitive files.
    refuse_symlink_unless(&path, false)?;

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create parent dir {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }
    }

    match client.format() {
        Format::Json => write_json_config(client, &path, binary_path),
        Format::Toml => write_toml_config(client, &path, binary_path),
    }
}

fn write_json_config(
    client: Client,
    path: &Path,
    binary_path: &Path,
) -> Result<InstallOutcome> {
    let root_key = client.json_root_key();
    let entry = graphatlas_json_entry(client, binary_path);

    if !path.exists() {
        let fresh = json!({
            root_key: { "graphatlas": entry },
        });
        write_json_atomic(path, &fresh)?;
        return Ok(InstallOutcome::Created {
            client,
            config_path: path.to_path_buf(),
        });
    }

    let bytes = fs::read(path).with_context(|| format!("read config {}", path.display()))?;
    let mut doc: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "config {} is corrupt JSON; refusing to overwrite. Fix or remove \
             the file manually.",
            path.display()
        )
    })?;

    let backup = backup_path_for(path);
    fs::write(&backup, &bytes).with_context(|| format!("write backup {}", backup.display()))?;

    let servers = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root must be a JSON object"))?
        .entry(root_key.to_string())
        .or_insert_with(|| json!({}));
    let servers_map = servers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`{root_key}` must be a JSON object"))?;
    let had_existing = servers_map.contains_key("graphatlas");
    servers_map.insert("graphatlas".to_string(), entry);

    write_json_atomic(path, &doc)?;

    Ok(InstallOutcome::Updated {
        client,
        config_path: path.to_path_buf(),
        had_existing_entry: had_existing,
    })
}

fn write_toml_config(
    client: Client,
    path: &Path,
    binary_path: &Path,
) -> Result<InstallOutcome> {
    let existed = path.exists();
    let mut doc = read_toml_or_empty(path)?;

    if existed {
        let backup = backup_path_for(path);
        if let Ok(bytes) = fs::read(path) {
            fs::write(&backup, &bytes)
                .with_context(|| format!("write backup {}", backup.display()))?;
        }
    }

    let servers = doc
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("`mcp_servers` must be a TOML table"))?;
    let had_existing = servers.contains_key("graphatlas");
    servers.insert("graphatlas".to_string(), graphatlas_toml_entry(binary_path));

    super::json_io::atomic_write_toml(path, &doc)?;

    Ok(if existed {
        InstallOutcome::Updated {
            client,
            config_path: path.to_path_buf(),
            had_existing_entry: had_existing,
        }
    } else {
        InstallOutcome::Created {
            client,
            config_path: path.to_path_buf(),
        }
    })
}

fn graphatlas_json_entry(client: Client, binary_path: &Path) -> Value {
    // Zed's context_servers entries require a `source: "custom"` discriminator.
    if client == Client::Zed {
        return json!({
            "source": "custom",
            "command": binary_path.to_string_lossy(),
            "args": ["mcp"],
        });
    }
    json!({
        "command": binary_path.to_string_lossy(),
        "args": ["mcp"],
    })
}

fn graphatlas_toml_entry(binary_path: &Path) -> toml::Value {
    let mut tbl = toml::value::Table::new();
    tbl.insert(
        "command".into(),
        toml::Value::String(binary_path.to_string_lossy().into_owned()),
    );
    tbl.insert(
        "args".into(),
        toml::Value::Array(vec![toml::Value::String("mcp".into())]),
    );
    toml::Value::Table(tbl)
}

fn default_path_for(client: Client) -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| {
        anyhow!(
            "HOME env var not set; cannot resolve default config path for \
             {:?}. Pass --config-path to override.",
            client
        )
    })?;
    Ok(client.default_config_path_for_home(Path::new(&home)))
}

fn backup_path_for(config: &Path) -> PathBuf {
    let mut s = config.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(value).context("serialize MCP config")?;
    atomic_write_bytes(path, &serialized)
}
