//! `graphatlas install --client <name>` — wire the MCP config for an LLM
//! client so the user doesn't have to hand-edit JSON.
//!
//! Behaviour (AS-005):
//!   1. Resolve default config path per client, or accept override.
//!   2. If config exists: parse as JSON, deep-merge our entry under
//!      `mcpServers.graphatlas`, preserve every other key.
//!   3. Write `.mcp.json.bak` (copy of old) BEFORE writing the new file.
//!   4. Atomic write: tmp → rename.
//!   5. If config was missing: create fresh `{"mcpServers": {"graphatlas":…}}`.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Claude,
    Cursor,
    Cline,
}

impl std::str::FromStr for Client {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "cursor" => Ok(Self::Cursor),
            "cline" => Ok(Self::Cline),
            other => Err(anyhow!(
                "unknown client `{other}` — supported: claude, cursor, cline"
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
        }
    }

    /// Default MCP config file path for each client, relative to `$HOME`.
    /// The on-disk format is JSON with a `mcpServers` top-level object.
    /// Callers can override via the `config_path` arg to [`write_mcp_config`]
    /// — primarily for tests and non-standard installs.
    pub fn default_config_relative(self) -> &'static str {
        match self {
            Self::Claude => ".claude/mcp.json",
            Self::Cursor => ".cursor/mcp.json",
            Self::Cline => ".config/cline/mcp.json",
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

/// Write the `graphatlas` entry into the MCP config for `client`.
///
/// - `config_path`: override path; when `None`, resolve from `$HOME` + default.
/// - `binary_path`: what to put in `command`. Typically `std::env::current_exe()`.
pub fn write_mcp_config(
    client: Client,
    config_path: Option<&Path>,
    binary_path: &Path,
) -> Result<InstallOutcome> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => default_path_for(client)?,
    };

    // If parent missing, create it with 0700 on Unix for consistency.
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

    if !path.exists() {
        let fresh = json!({
            "mcpServers": {
                "graphatlas": graphatlas_entry(binary_path),
            }
        });
        write_json_atomic(&path, &fresh)?;
        return Ok(InstallOutcome::Created {
            client,
            config_path: path,
        });
    }

    // Parse existing config. Do NOT clobber on corrupt JSON — surface a typed
    // error so users can inspect + recover.
    let bytes = fs::read(&path).with_context(|| format!("read config {}", path.display()))?;
    let mut doc: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "config {} is corrupt JSON; refusing to overwrite. Fix or remove \
             the file manually.",
            path.display()
        )
    })?;

    // Backup original bytes before touching anything.
    let backup = backup_path_for(&path);
    fs::write(&backup, &bytes).with_context(|| format!("write backup {}", backup.display()))?;

    // Merge: ensure `mcpServers` is an object, add/replace `graphatlas`.
    let servers = doc
        .as_object_mut()
        .ok_or_else(|| anyhow!("config root must be a JSON object"))?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let servers_map = servers
        .as_object_mut()
        .ok_or_else(|| anyhow!("`mcpServers` must be a JSON object"))?;
    let had_existing = servers_map.contains_key("graphatlas");
    servers_map.insert("graphatlas".to_string(), graphatlas_entry(binary_path));

    write_json_atomic(&path, &doc)?;

    Ok(InstallOutcome::Updated {
        client,
        config_path: path,
        had_existing_entry: had_existing,
    })
}

fn graphatlas_entry(binary_path: &Path) -> Value {
    json!({
        "command": binary_path.to_string_lossy(),
        "args": ["mcp"],
    })
}

fn default_path_for(client: Client) -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| {
        anyhow!(
            "HOME env var not set; cannot resolve default config path for \
             {:?}. Pass --config-path to override.",
            client
        )
    })?;
    Ok(PathBuf::from(home).join(client.default_config_relative()))
}

fn backup_path_for(config: &Path) -> PathBuf {
    let mut s = config.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(value).context("serialize MCP config")?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("mcp")
    ));
    fs::write(&tmp, &serialized).with_context(|| format!("write tmp {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
