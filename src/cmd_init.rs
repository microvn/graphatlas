//! `ga init` — drop the GraphAtlas Claude Code skill + CLAUDE.md
//! snippet + MCP allow-list (and optionally a SessionStart hook) into
//! the current project so the agent prefers `ga_*` over Grep/Bash for
//! code navigation.
//!
//! UX modes:
//! - **Interactive** (default when no flags + stdin is a TTY): prompts
//!   y/n per component with a pre-flight state (will create / update /
//!   unchanged).
//! - **Non-interactive** (`--yes`, `--all`, any `--with-*` flag, or
//!   non-TTY stdin): proceeds without prompting; component selection
//!   comes from flags (defaults: all-except-hook).

use crate::install::{
    claudemd::{install_claudemd, uninstall_claudemd, ClaudeMdOutcome},
    permissions::{install_permissions, uninstall_permissions, PermissionsOutcome},
    session_hook::{install_session_hook, uninstall_session_hook, SessionHookOutcome},
    skill::{install_skill, uninstall_skill, SkillOutcome, SKILL_REL_PATH},
};
use anyhow::{anyhow, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct InitOptions {
    pub project_root: Option<PathBuf>,
    pub with_skill: bool,
    pub with_claudemd: bool,
    pub with_permissions: bool,
    pub with_hook: bool,
    pub all: bool,
    pub remove_hook: bool,
    pub yes: bool,
    pub binary_path: Option<PathBuf>,
}

impl InitOptions {
    fn explicit_track_flag(&self) -> bool {
        self.with_skill
            || self.with_claudemd
            || self.with_permissions
            || self.with_hook
            || self.all
    }

    fn resolved_flags(&self) -> ResolvedFlags {
        // Default behaviour when --yes or non-TTY: install skill +
        // claudemd + permissions, NOT the hook (hook stays opt-in).
        let any_track = self.with_skill || self.with_claudemd || self.with_permissions;
        let default_all = !any_track && !self.remove_hook;
        ResolvedFlags {
            skill: self.with_skill || self.all || default_all,
            claudemd: self.with_claudemd || self.all || default_all,
            permissions: self.with_permissions || self.all || default_all,
            hook: self.with_hook || self.all,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedFlags {
    pub skill: bool,
    pub claudemd: bool,
    pub permissions: bool,
    pub hook: bool,
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

    let flags = if should_run_interactive(&opts) {
        match interactive_pick(&project_root)? {
            Some(f) => f,
            None => {
                println!("Aborted. No changes written.");
                return Ok(());
            }
        }
    } else {
        opts.resolved_flags()
    };

    println!("graphatlas init: {}", project_root.display());
    apply(&project_root, flags, opts.binary_path.as_deref(), opts.all)?;
    Ok(())
}

fn apply(
    project_root: &Path,
    flags: ResolvedFlags,
    binary_override: Option<&Path>,
    all: bool,
) -> Result<()> {
    if flags.skill {
        report_skill(&install_skill(project_root)?);
    }
    if flags.claudemd {
        report_claudemd(&install_claudemd(project_root)?);
    }
    if flags.permissions {
        report_permissions(&install_permissions(project_root)?);
    }
    if flags.hook {
        let binary = match binary_override {
            Some(p) => p.to_path_buf(),
            None => std::env::current_exe()?,
        };
        report_session_hook(&install_session_hook(project_root, &binary)?);
    } else if !all {
        println!(
            "  (SessionStart hook NOT installed; rerun with --with-hook to enable \
             the per-session discovery reminder.)"
        );
    }
    Ok(())
}

pub fn cmd_init_uninstall_all(project_root: Option<&Path>) -> Result<()> {
    let project_root = resolve_project_root(project_root)?;
    let skill = uninstall_skill(&project_root)?;
    let claudemd = uninstall_claudemd(&project_root)?;
    let perms = uninstall_permissions(&project_root)?;
    let hook = uninstall_session_hook(&project_root)?;
    println!(
        "Removed: skill={skill} claudemd={claudemd} permissions={perms} session_hook={hook}",
    );
    Ok(())
}

fn resolve_project_root(arg: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = arg {
        return Ok(p.to_path_buf());
    }
    Ok(std::env::current_dir()?)
}

// -------------------------------------------------------------------
// Interactive flow
// -------------------------------------------------------------------

fn should_run_interactive(opts: &InitOptions) -> bool {
    if opts.yes || opts.explicit_track_flag() {
        return false;
    }
    std::io::stdin().is_terminal()
}

/// Walk the user through component selection. Returns `Some(flags)`
/// to apply, or `None` if the user declined at the final confirm.
fn interactive_pick(project_root: &Path) -> Result<Option<ResolvedFlags>> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    interactive_pick_io(project_root, &mut stdin, &mut stdout)
}

/// Test-injectable core of the interactive flow. `reader` supplies
/// answer lines (one per prompt); `writer` captures rendered output.
pub fn interactive_pick_io<R: BufRead, W: Write>(
    project_root: &Path,
    reader: &mut R,
    writer: &mut W,
) -> Result<Option<ResolvedFlags>> {
    writeln!(writer, "graphatlas init — interactive setup")?;
    writeln!(writer, "  project: {}", project_root.display())?;
    writeln!(writer)?;
    writeln!(
        writer,
        "Configure Claude Code to prefer ga_* MCP tools over Grep/Bash for \
         code navigation in this repo. Choose which components to install."
    )?;
    writeln!(writer)?;

    let skill = prompt_component(
        reader,
        writer,
        "Skill file",
        ".claude/skills/graphatlas.md",
        "Routing skill loaded by Claude Code on every turn.",
        preflight_skill(project_root),
        true,
    )?;

    let claudemd = prompt_component(
        reader,
        writer,
        "CLAUDE.md block",
        "CLAUDE.md (managed block)",
        "Always-on context hint pointing at the skill + ga_* tools.",
        preflight_claudemd(project_root),
        true,
    )?;

    let permissions = prompt_component(
        reader,
        writer,
        "MCP allow-list",
        ".claude/settings.json (permissions.allow)",
        "Auto-approve mcp__graphatlas__* calls (no per-tool prompt).",
        preflight_permissions(project_root),
        true,
    )?;

    let hook = prompt_component(
        reader,
        writer,
        "SessionStart hook",
        ".claude/settings.json (hooks.SessionStart)",
        "Inject a discovery-protocol reminder once per Claude Code session.",
        preflight_hook(project_root),
        false, // opt-in default
    )?;

    writeln!(writer)?;
    writeln!(writer, "Summary:")?;
    render_pick(writer, "skill", skill)?;
    render_pick(writer, "CLAUDE.md", claudemd)?;
    render_pick(writer, "permissions", permissions)?;
    render_pick(writer, "SessionStart hook", hook)?;
    writeln!(writer)?;

    if !skill && !claudemd && !permissions && !hook {
        writeln!(writer, "Nothing selected — aborting.")?;
        return Ok(None);
    }

    let proceed = prompt_yes_no(reader, writer, "Proceed?", true)?;
    if !proceed {
        return Ok(None);
    }
    writeln!(writer)?;
    Ok(Some(ResolvedFlags {
        skill,
        claudemd,
        permissions,
        hook,
    }))
}

fn render_pick<W: Write>(writer: &mut W, label: &str, on: bool) -> Result<()> {
    let glyph = if on { "✓" } else { "✗" };
    writeln!(writer, "  {glyph} {label}")?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prompt_component<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    short: &str,
    target: &str,
    rationale: &str,
    preflight: &str,
    default_yes: bool,
) -> Result<bool> {
    writeln!(writer, "{short} — {target}")?;
    writeln!(writer, "  {rationale}")?;
    writeln!(writer, "  state: {preflight}")?;
    let on = prompt_yes_no(reader, writer, "  install?", default_yes)?;
    writeln!(writer)?;
    Ok(on)
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
            // EOF — fall back to default rather than loop forever.
            writeln!(writer)?;
            return Ok(default_yes);
        }
        match buf.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default_yes),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            other => {
                writeln!(writer, "  (please answer y or n; got {other:?})")?;
            }
        }
    }
}

// -------------------------------------------------------------------
// Pre-flight state — what would the install do?
// -------------------------------------------------------------------

fn preflight_skill(project_root: &Path) -> &'static str {
    let path = project_root.join(SKILL_REL_PATH);
    if !path.exists() {
        "missing — will create"
    } else {
        "exists — will refresh to current version"
    }
}

fn preflight_claudemd(project_root: &Path) -> &'static str {
    let path = project_root.join("CLAUDE.md");
    if !path.exists() {
        "CLAUDE.md missing — will create"
    } else if std::fs::read_to_string(&path)
        .map(|s| s.contains(crate::install::claudemd::BEGIN_MARKER))
        .unwrap_or(false)
    {
        "GA block present — will update if needed"
    } else {
        "CLAUDE.md exists — will append GA block"
    }
}

fn preflight_permissions(project_root: &Path) -> &'static str {
    let path = project_root.join(".claude").join("settings.json");
    if !path.exists() {
        "settings.json missing — will create"
    } else {
        "settings.json exists — will merge allow-list"
    }
}

fn preflight_hook(project_root: &Path) -> &'static str {
    let path = project_root.join(".claude").join("settings.json");
    if !path.exists() {
        "no hook installed — will create"
    } else if std::fs::read_to_string(&path)
        .map(|s| s.contains("\"_managed_by\""))
        .unwrap_or(false)
    {
        "managed hook present — will refresh"
    } else {
        "no managed hook — will add"
    }
}

// -------------------------------------------------------------------
// Per-outcome reporters
// -------------------------------------------------------------------

fn report_skill(outcome: &SkillOutcome) {
    match outcome {
        SkillOutcome::Created(p) => println!("✓ Created skill {}", p.display()),
        SkillOutcome::Updated(p) => println!("✓ Updated skill {}", p.display()),
        SkillOutcome::Unchanged(p) => println!("- Skill up to date {}", p.display()),
    }
}

fn report_claudemd(outcome: &ClaudeMdOutcome) {
    match outcome {
        ClaudeMdOutcome::Created(p) => println!("✓ Created {}", p.display()),
        ClaudeMdOutcome::BlockAdded(p) => println!("✓ Added GA block to {}", p.display()),
        ClaudeMdOutcome::BlockReplaced(p) => println!("✓ Updated GA block in {}", p.display()),
        ClaudeMdOutcome::Unchanged(p) => {
            println!("- CLAUDE.md GA block up to date {}", p.display())
        }
    }
}

fn report_permissions(outcome: &PermissionsOutcome) {
    match outcome {
        PermissionsOutcome::Created(p) => {
            println!("✓ Created {} with GA permissions", p.display())
        }
        PermissionsOutcome::Added(p) => println!("✓ Added GA permissions to {}", p.display()),
        PermissionsOutcome::AlreadyPresent(p) => {
            println!("- GA permissions already present in {}", p.display())
        }
    }
}

fn report_session_hook(outcome: &SessionHookOutcome) {
    match outcome {
        SessionHookOutcome::Created(p) => {
            println!("✓ Created {} with SessionStart hook", p.display())
        }
        SessionHookOutcome::Added(p) => println!("✓ Added SessionStart hook to {}", p.display()),
        SessionHookOutcome::Replaced(p) => {
            println!("✓ Refreshed SessionStart hook in {}", p.display())
        }
        SessionHookOutcome::AlreadyPresent(p) => {
            println!("- SessionStart hook already present in {}", p.display())
        }
    }
}
