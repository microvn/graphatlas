//! `ga init` — multi-platform LLM-agent setup. Drops MCP server entries
//! + instruction files into the project for each selected platform so
//! agents prefer `ga_*` over Grep/Bash for code navigation.
//!
//! Supported platforms:
//! - Claude Code: skill + CLAUDE.md block + permissions + (opt-in) SessionStart hook
//! - Cursor:      project-local MCP + `.cursor/rules/graphatlas.mdc`
//! - Cline:       per-OS VS Code extension MCP + `.clinerules`
//! - Codex CLI:   `~/.codex/config.toml` MCP + `AGENTS.md` block
//! - Gemini CLI:  project-local `.gemini/settings.json` MCP + `GEMINI.md` block
//!
//! UX modes:
//! - **Interactive** (no flags + TTY): numbered-toggle multi-select with
//!   detected agents pre-checked, then hook opt-in + confirm.
//! - **Positional** (`ga init claude cursor`): explicit platform list.
//! - **`--all`**: install for all 5 platforms regardless of detect.
//! - **`--yes`** / non-TTY: skip prompts, use detected (default: claude-code if none).

use crate::install::{
    claudemd::{install_claudemd, uninstall_claudemd, ClaudeMdOutcome},
    hook::{install_hook_with_binary, HookOutcome},
    mcp_config::{write_mcp_config, Client},
    permissions::{install_permissions, uninstall_permissions, PermissionsOutcome},
    platforms::{InstructionOutcome, Platform, SimpleOutcome},
    session_hook::{install_session_hook, uninstall_session_hook, SessionHookOutcome},
    skill::{install_skill, uninstall_skill, SkillOutcome},
};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct InitOptions {
    pub project_root: Option<PathBuf>,
    /// Explicit platform list (from positional args or --platforms).
    /// Empty → interactive picker (TTY) or detected-default (non-TTY).
    pub platforms: Vec<Platform>,
    /// Install for every supported platform regardless of detection.
    pub all: bool,
    /// Skip interactive prompts. `yes + no platforms` defaults to
    /// claude-code only (backward-compat with v1 `ga init`).
    pub yes: bool,
    /// Install the Claude Code SessionStart discovery-reminder hook.
    pub with_hook: bool,
    /// Remove the managed SessionStart hook (Claude Code only).
    pub remove_hook: bool,
    /// Skip auto-install of the PostToolUse reindex hook for platforms
    /// that support it (claude-code, cursor, codex). Default = false,
    /// i.e. reindex hook IS installed alongside MCP + instructions.
    pub no_reindex_hook: bool,
    /// Binary path written into MCP entries. None → `current_exe()`.
    pub binary_path: Option<PathBuf>,
}

pub fn cmd_init(opts: InitOptions) -> Result<()> {
    let project_root = resolve_project_root(opts.project_root.as_deref())?;

    if opts.remove_hook {
        let removed = uninstall_session_hook(&project_root)?;
        if removed {
            println!("✓ Removed GraphAtlas SessionStart hook from .claude/settings.json");
        } else {
            println!("- No managed SessionStart hook entry to remove.");
        }
        return Ok(());
    }

    let (platforms, with_hook) = resolve_platforms(&opts)?;
    if platforms.is_empty() {
        println!("No platforms selected. Aborted.");
        return Ok(());
    }

    println!("graphatlas init: {}", project_root.display());
    for p in &platforms {
        println!("  → {}", p.display());
        apply_platform(*p, &project_root, &opts, with_hook && *p == Platform::ClaudeCode)?;
    }
    Ok(())
}

fn resolve_platforms(opts: &InitOptions) -> Result<(Vec<Platform>, bool)> {
    if opts.all {
        return Ok((Platform::ALL.to_vec(), opts.with_hook));
    }
    if !opts.platforms.is_empty() {
        return Ok((dedup_keep_order(&opts.platforms), opts.with_hook));
    }
    if opts.yes || !std::io::stdin().is_terminal() {
        // Non-interactive default — preserves v1 behaviour (Claude Code only).
        let detected: Vec<Platform> = Platform::ALL
            .iter()
            .copied()
            .filter(Platform::detect)
            .collect();
        let chosen = if detected.is_empty() {
            vec![Platform::ClaudeCode]
        } else {
            detected
        };
        return Ok((chosen, opts.with_hook));
    }
    // Interactive picker.
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let detected: HashSet<Platform> = Platform::ALL
        .iter()
        .copied()
        .filter(Platform::detect)
        .collect();
    let picked = interactive_pick_platforms_io(&detected, &mut stdin, &mut stdout)?;
    if let Some((plats, hook)) = picked {
        Ok((plats, hook))
    } else {
        Ok((Vec::new(), false))
    }
}

fn apply_platform(
    platform: Platform,
    project_root: &Path,
    opts: &InitOptions,
    install_claude_hook: bool,
) -> Result<()> {
    match platform {
        Platform::ClaudeCode => apply_claude_code(project_root, opts, install_claude_hook),
        Platform::Cursor => apply_with_mcp(platform, Client::Cursor, project_root, opts),
        Platform::Cline => apply_with_mcp(platform, Client::Cline, project_root, opts),
        Platform::CodexCli => apply_with_mcp(platform, Client::Codex, project_root, opts),
        Platform::GeminiCli => apply_with_mcp(platform, Client::Gemini, project_root, opts),
        Platform::Windsurf => apply_with_mcp(platform, Client::Windsurf, project_root, opts),
        Platform::Continue => apply_with_mcp(platform, Client::Continue, project_root, opts),
        Platform::Zed => apply_with_mcp(platform, Client::Zed, project_root, opts),
    }
}

fn apply_claude_code(
    project_root: &Path,
    opts: &InitOptions,
    install_session_hook_flag: bool,
) -> Result<()> {
    report_skill(&install_skill(project_root)?);
    report_claudemd(&install_claudemd(project_root)?);
    report_permissions(&install_permissions(project_root)?);
    install_reindex_hook_if_supported(Platform::ClaudeCode, project_root, opts)?;
    if install_session_hook_flag {
        let binary = resolve_binary(opts)?;
        report_session_hook(&install_session_hook(project_root, &binary)?);
    } else {
        println!(
            "    (SessionStart hook NOT installed; pass --with-hook to enable.)"
        );
    }
    Ok(())
}

fn apply_with_mcp(
    platform: Platform,
    client: Client,
    project_root: &Path,
    opts: &InitOptions,
) -> Result<()> {
    let binary = resolve_binary(opts)?;
    let mcp_path = client.project_config_path(project_root);
    let outcome = write_mcp_config(client, mcp_path.as_deref(), &binary)?;
    report_mcp(&outcome);

    let instr = platform.install_instructions(project_root)?;
    report_instruction(&instr);

    install_reindex_hook_if_supported(platform, project_root, opts)?;
    Ok(())
}

/// Install the PostToolUse reindex hook for platforms that support it
/// (Claude Code / Cursor / Codex). Silent no-op for the other 5
/// platforms — they rely on the agent to invoke `ga_reindex` itself.
fn install_reindex_hook_if_supported(
    platform: Platform,
    project_root: &Path,
    opts: &InitOptions,
) -> Result<()> {
    if opts.no_reindex_hook {
        return Ok(());
    }
    let Some(hook_client) = platform.hook_client() else {
        return Ok(());
    };
    let binary = resolve_binary(opts)?;
    let outcome = install_hook_with_binary(hook_client, project_root, &binary, false)?;
    report_reindex_hook(&outcome);
    Ok(())
}

fn report_reindex_hook(outcome: &HookOutcome) {
    match outcome {
        HookOutcome::Created { path, .. } => {
            println!("    ✓ Created reindex hook in {}", path.display())
        }
        HookOutcome::Added { path, .. } => {
            println!("    ✓ Added reindex hook to {}", path.display())
        }
        HookOutcome::AlreadyPresent { path, .. } => {
            println!("    - Reindex hook already present in {}", path.display())
        }
    }
}

fn resolve_binary(opts: &InitOptions) -> Result<PathBuf> {
    Ok(match &opts.binary_path {
        Some(p) => p.clone(),
        None => std::env::current_exe()?,
    })
}

pub fn cmd_init_uninstall_all(project_root: Option<&Path>) -> Result<()> {
    let project_root = resolve_project_root(project_root)?;
    let skill = uninstall_skill(&project_root)?;
    let claudemd = uninstall_claudemd(&project_root)?;
    let perms = uninstall_permissions(&project_root)?;
    let hook = uninstall_session_hook(&project_root)?;
    let mut platform_removed = Vec::new();
    for p in Platform::ALL {
        if *p == Platform::ClaudeCode {
            continue;
        }
        if p.uninstall_instructions(&project_root)? {
            platform_removed.push(p.slug());
        }
    }
    println!(
        "Removed: skill={skill} claudemd={claudemd} permissions={perms} session_hook={hook} \
         platforms={platform_removed:?}",
    );
    Ok(())
}

fn resolve_project_root(arg: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = arg {
        return Ok(p.to_path_buf());
    }
    Ok(std::env::current_dir()?)
}

fn dedup_keep_order(list: &[Platform]) -> Vec<Platform> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(list.len());
    for &p in list {
        if seen.insert(p) {
            out.push(p);
        }
    }
    out
}

// -------------------------------------------------------------------
// Interactive multi-select picker (numbered toggle)
// -------------------------------------------------------------------

/// Returns `Some((platforms, with_hook))` on confirm, `None` if user
/// aborted or selected nothing.
pub fn interactive_pick_platforms_io<R: BufRead, W: Write>(
    detected: &HashSet<Platform>,
    reader: &mut R,
    writer: &mut W,
) -> Result<Option<(Vec<Platform>, bool)>> {
    writeln!(writer, "graphatlas init — interactive setup")?;
    writeln!(writer)?;
    let detected_names: Vec<&str> = Platform::ALL
        .iter()
        .filter(|p| detected.contains(p))
        .map(|p| p.slug())
        .collect();
    let not_detected_names: Vec<&str> = Platform::ALL
        .iter()
        .filter(|p| !detected.contains(p))
        .map(|p| p.slug())
        .collect();
    if detected_names.is_empty() {
        writeln!(writer, "  detected: (none)")?;
    } else {
        writeln!(writer, "  detected:     {}", detected_names.join(", "))?;
    }
    if !not_detected_names.is_empty() {
        writeln!(writer, "  not detected: {}", not_detected_names.join(", "))?;
    }
    writeln!(writer)?;
    writeln!(
        writer,
        "Toggle by number, \"done\" / Enter to confirm, \"all\" / \"none\" shortcuts."
    )?;

    // Selection state: detected agents pre-checked.
    let mut selected: Vec<bool> = Platform::ALL.iter().map(|p| detected.contains(p)).collect();
    render_platform_list(writer, &selected)?;

    loop {
        write!(writer, "> ")?;
        writer.flush()?;
        let mut buf = String::new();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            // EOF — confirm with current selection.
            break;
        }
        let trimmed = buf.trim();
        match parse_toggle_command(trimmed, Platform::ALL.len()) {
            Ok(ToggleCmd::Done) => break,
            Ok(ToggleCmd::All) => {
                for s in selected.iter_mut() {
                    *s = true;
                }
                render_platform_list(writer, &selected)?;
            }
            Ok(ToggleCmd::None) => {
                for s in selected.iter_mut() {
                    *s = false;
                }
                render_platform_list(writer, &selected)?;
            }
            Ok(ToggleCmd::Toggle(indices)) => {
                for i in indices {
                    selected[i] = !selected[i];
                }
                render_platform_list(writer, &selected)?;
            }
            Err(msg) => {
                writeln!(writer, "  ({msg})")?;
            }
        }
    }

    let picked: Vec<Platform> = Platform::ALL
        .iter()
        .zip(selected.iter())
        .filter_map(|(p, on)| if *on { Some(*p) } else { None })
        .collect();
    if picked.is_empty() {
        writeln!(writer, "Nothing selected — aborting.")?;
        return Ok(None);
    }

    writeln!(writer)?;
    writeln!(
        writer,
        "Selected: {}",
        picked
            .iter()
            .map(|p| p.slug())
            .collect::<Vec<_>>()
            .join(", ")
    )?;
    let with_hook = if picked.contains(&Platform::ClaudeCode) {
        prompt_yes_no(
            reader,
            writer,
            "Install Claude Code SessionStart hook? (opt-in)",
            false,
        )?
    } else {
        false
    };

    writeln!(writer)?;
    writeln!(writer, "Will install:")?;
    for p in &picked {
        let summary = match p {
            Platform::ClaudeCode => "skill + CLAUDE.md block + permissions"
                .to_string()
                + if with_hook { " + SessionStart hook" } else { "" },
            Platform::Cursor => "MCP register + .cursor/rules/graphatlas.mdc".to_string(),
            Platform::Cline => "MCP register + .clinerules".to_string(),
            Platform::CodexCli => "MCP register (~/.codex/config.toml) + AGENTS.md block".to_string(),
            Platform::GeminiCli => "MCP register + GEMINI.md block".to_string(),
            Platform::Windsurf => "MCP register + .windsurfrules".to_string(),
            Platform::Continue => "MCP register (.continue/mcpServers/graphatlas.json)".to_string(),
            Platform::Zed => "MCP register (.zed/settings.json context_servers)".to_string(),
        };
        writeln!(writer, "  {} → {summary}", p.slug())?;
    }
    writeln!(writer)?;
    let proceed = prompt_yes_no(reader, writer, "Proceed?", true)?;
    if !proceed {
        return Ok(None);
    }
    Ok(Some((picked, with_hook)))
}

fn render_platform_list<W: Write>(writer: &mut W, selected: &[bool]) -> Result<()> {
    for (i, p) in Platform::ALL.iter().enumerate() {
        let mark = if selected[i] { "x" } else { " " };
        writeln!(writer, "  [{mark}] {}) {}", i + 1, p.slug())?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum ToggleCmd {
    Toggle(Vec<usize>),
    All,
    None,
    Done,
}

fn parse_toggle_command(input: &str, max: usize) -> std::result::Result<ToggleCmd, String> {
    let lowered = input.trim().to_ascii_lowercase();
    match lowered.as_str() {
        "" | "done" | "d" => return Ok(ToggleCmd::Done),
        "all" | "a" => return Ok(ToggleCmd::All),
        "none" | "clear" => return Ok(ToggleCmd::None),
        _ => {}
    }
    let mut indices = Vec::new();
    for token in lowered.split_whitespace() {
        // Allow comma separators too: "2,4" or "2, 4".
        for piece in token.split(',') {
            if piece.is_empty() {
                continue;
            }
            let n: usize = piece.parse().map_err(|_| {
                format!("expected number / done / all / none; got {piece:?}")
            })?;
            if n == 0 || n > max {
                return Err(format!("number must be 1..{max}; got {n}"));
            }
            indices.push(n - 1);
        }
    }
    if indices.is_empty() {
        return Err("no numbers parsed".into());
    }
    Ok(ToggleCmd::Toggle(indices))
}

fn prompt_yes_no<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    question: &str,
    default_yes: bool,
) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        write!(writer, "{question} {hint}: ")?;
        writer.flush()?;
        let mut buf = String::new();
        let n = reader
            .read_line(&mut buf)
            .map_err(|e| anyhow!("read stdin: {e}"))?;
        if n == 0 {
            writeln!(writer)?;
            return Ok(default_yes);
        }
        match buf.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            other => writeln!(writer, "  (please answer y or n; got {other:?})")?,
        }
    }
}

// -------------------------------------------------------------------
// Reporters
// -------------------------------------------------------------------

fn report_skill(outcome: &SkillOutcome) {
    match outcome {
        SkillOutcome::Created(p) => println!("    ✓ Created skill {}", p.display()),
        SkillOutcome::Updated(p) => println!("    ✓ Updated skill {}", p.display()),
        SkillOutcome::Unchanged(p) => println!("    - Skill up to date {}", p.display()),
    }
}

fn report_claudemd(outcome: &ClaudeMdOutcome) {
    match outcome {
        ClaudeMdOutcome::Created(p) => println!("    ✓ Created {}", p.display()),
        ClaudeMdOutcome::BlockAdded(p) => println!("    ✓ Added GA block to {}", p.display()),
        ClaudeMdOutcome::BlockReplaced(p) => println!("    ✓ Updated GA block in {}", p.display()),
        ClaudeMdOutcome::Unchanged(p) => {
            println!("    - CLAUDE.md GA block up to date {}", p.display())
        }
    }
}

fn report_permissions(outcome: &PermissionsOutcome) {
    match outcome {
        PermissionsOutcome::Created(p) => {
            println!("    ✓ Created {} with GA permissions", p.display())
        }
        PermissionsOutcome::Added(p) => println!("    ✓ Added GA permissions to {}", p.display()),
        PermissionsOutcome::AlreadyPresent(p) => {
            println!("    - GA permissions already present in {}", p.display())
        }
    }
}

fn report_session_hook(outcome: &SessionHookOutcome) {
    match outcome {
        SessionHookOutcome::Created(p) => {
            println!("    ✓ Created {} with SessionStart hook", p.display())
        }
        SessionHookOutcome::Added(p) => {
            println!("    ✓ Added SessionStart hook to {}", p.display())
        }
        SessionHookOutcome::Replaced(p) => {
            println!("    ✓ Refreshed SessionStart hook in {}", p.display())
        }
        SessionHookOutcome::AlreadyPresent(p) => {
            println!("    - SessionStart hook already present in {}", p.display())
        }
    }
}

fn report_mcp(outcome: &crate::install::mcp_config::InstallOutcome) {
    use crate::install::mcp_config::InstallOutcome as O;
    match outcome {
        O::Created { config_path, .. } => println!("    ✓ Created MCP entry in {}", config_path.display()),
        O::Updated {
            config_path,
            had_existing_entry,
            ..
        } => {
            if *had_existing_entry {
                println!("    ✓ Refreshed MCP entry in {}", config_path.display());
            } else {
                println!("    ✓ Added MCP entry to {}", config_path.display());
            }
        }
    }
}

fn report_instruction(outcome: &InstructionOutcome) {
    let simple = match outcome {
        InstructionOutcome::DelegatedToClaudeCodeFlow => return,
        InstructionOutcome::McpOnly => return,
        InstructionOutcome::Cursor(s) => s,
        InstructionOutcome::Cline(s) => s,
        InstructionOutcome::Codex(s) => s,
        InstructionOutcome::Gemini(s) => s,
        InstructionOutcome::Windsurf(s) => s,
    };
    match simple {
        SimpleOutcome::Created(p) => println!("    ✓ Created {}", p.display()),
        SimpleOutcome::Updated(p) => println!("    ✓ Updated {}", p.display()),
        SimpleOutcome::Unchanged(p) => println!("    - Unchanged {}", p.display()),
    }
}
