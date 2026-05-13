//! v0.2 install subcommand dispatch — splits cmd_install + cmd_install_hook
//! out of `main.rs` to keep the entry binary under the 350 LOC threshold.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::str::FromStr;

pub fn cmd_install(
    client_arg: Option<String>,
    config_path: Option<PathBuf>,
    hook_arg: Option<String>,
    project_root: Option<PathBuf>,
    uninstall: bool,
    verify: bool,
    follow_symlinks: bool,
) -> Result<()> {
    if let Some(hook_str) = hook_arg {
        return cmd_install_hook(hook_str, project_root, uninstall, verify, follow_symlinks);
    }
    let client_str = client_arg.ok_or_else(|| {
        anyhow::anyhow!(
            "--client or --hook required. `--client {{claude|cursor|cline}}` wires the \
             MCP server entry; `--hook {{claude-code|cursor|codex}}` wires the \
             PostToolUse trigger that auto-calls ga_reindex. \
             Run `graphatlas install --help` for examples."
        )
    })?;
    let client = graphatlas::install::Client::from_str(&client_str)?;
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("graphatlas"));
    let outcome = graphatlas::install::write_mcp_config(client, config_path.as_deref(), &bin)?;
    match outcome {
        graphatlas::install::InstallOutcome::Created {
            config_path,
            client,
        } => {
            println!(
                "✓ created MCP config for {} at {}",
                client.as_str(),
                config_path.display()
            );
        }
        graphatlas::install::InstallOutcome::Updated {
            config_path,
            client,
            had_existing_entry,
        } => {
            let verb = if had_existing_entry {
                "refreshed"
            } else {
                "added"
            };
            println!(
                "✓ {verb} graphatlas entry in {} MCP config at {}",
                client.as_str(),
                config_path.display()
            );
            println!("  backup saved to {}.bak", config_path.display());
        }
    }
    println!("\nNext: restart your MCP client to pick up the new config.");
    Ok(())
}


pub fn cmd_install_hook(
    hook_str: String,
    project_root: Option<PathBuf>,
    uninstall: bool,
    verify: bool,
    follow_symlinks: bool,
) -> Result<()> {
    use graphatlas::install::{
        install_hook, uninstall_hook, verify_hook, HookClient, HookOutcome, VerifyOutcome,
    };
    let client = HookClient::from_str(&hook_str)?;
    let root = match project_root {
        Some(p) => p,
        None => std::env::current_dir().context("std::env::current_dir for --hook")?,
    };

    if verify {
        match verify_hook(client, &root)? {
            VerifyOutcome::Ok => {
                println!("✓ {} hook installed correctly", client.as_str());
                Ok(())
            }
            VerifyOutcome::Missing { hint } => Err(anyhow::anyhow!(
                "hook missing for {}: {hint}",
                client.as_str()
            )),
            VerifyOutcome::Malformed { hint } => Err(anyhow::anyhow!(
                "hook malformed for {}: {hint}",
                client.as_str()
            )),
        }
    } else if uninstall {
        uninstall_hook(client, &root, follow_symlinks)?;
        println!("✓ removed {} hook entry", client.as_str());
        Ok(())
    } else {
        match install_hook(client, &root, follow_symlinks)? {
            HookOutcome::Created { path, .. } => {
                println!("✓ created {} hook at {}", client.as_str(), path.display());
            }
            HookOutcome::Added { path, .. } => {
                println!("✓ added GA hook to {} config at {}", client.as_str(), path.display());
            }
            HookOutcome::AlreadyPresent { path, .. } => {
                println!(
                    "✓ {} hook already installed at {} (no-op)",
                    client.as_str(),
                    path.display()
                );
            }
        }
        Ok(())
    }
}
