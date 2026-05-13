//! graphatlas CLI entry point.
//!
//! Foundation-C6 locks 8 subcommands for v1: mcp | init | doctor | install |
//! list | bench | update | cache. Each subcommand has its own `--help` with
//! examples per AS-019.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod cmd_install;
use cmd_install::cmd_install;

#[derive(Parser)]
#[command(
    name = "graphatlas",
    version = concat!(env!("CARGO_PKG_VERSION"), " (S-002 scaffold)"),
    about = "OSS Rust-native MCP server for code-analysis tools",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum HookSubcommand {
    /// Print the SessionStart discovery-protocol reminder to stdout.
    SessionStart,
}

#[derive(Subcommand)]
enum Command {
    /// Run MCP server over stdio (S-006).
    #[command(long_about = "\
Start the MCP (Model Context Protocol) server over stdin/stdout.
LLM clients (Claude Code, Cursor, Cline) launch this as a subprocess
and exchange JSON-RPC 2.0 requests conforming to MCP spec 2025-11-25.

EXAMPLES:
  graphatlas mcp                  # Normally invoked BY a client, not directly.
  echo '<jsonrpc request>' | graphatlas mcp   # Manual smoke test.
")]
    Mcp,

    /// Drop the GraphAtlas Claude Code skill + CLAUDE.md snippet +
    /// MCP allow-list (and optionally a SessionStart hook) into the
    /// current project so LLM agents prefer `ga_*` over Grep/Bash.
    #[command(long_about = "\
Wire GraphAtlas into the current project for Claude Code:
  • .claude/skills/graphatlas.md   — routing skill loaded by the agent.
  • CLAUDE.md (managed block)      — code-navigation hint, always-on context.
  • .claude/settings.json          — allow-list for mcp__graphatlas__* tools.
  • [opt-in] SessionStart hook     — discovery protocol injected per session.

EXAMPLES:
  graphatlas init                       # skill + CLAUDE.md + permissions (no hook)
  graphatlas init --with-hook           # also install SessionStart hook
  graphatlas init --all                 # everything
  graphatlas init --remove-hook         # remove the managed SessionStart hook
  graphatlas init --project-root /repo  # operate on a specific path
")]
    Init {
        /// Install the .claude/skills/graphatlas.md skill file.
        #[arg(long)]
        with_skill: bool,
        /// Append the managed code-navigation block to CLAUDE.md.
        #[arg(long)]
        with_claudemd: bool,
        /// Merge mcp__graphatlas__* into .claude/settings.json permissions.
        #[arg(long)]
        with_permissions: bool,
        /// Install the SessionStart discovery-reminder hook (opt-in).
        #[arg(long)]
        with_hook: bool,
        /// Install every component including the SessionStart hook.
        #[arg(long)]
        all: bool,
        /// Remove only the managed SessionStart hook entry. Preserves
        /// every other entry in .claude/settings.json.
        #[arg(long)]
        remove_hook: bool,
        /// Skip the interactive prompt and use defaults (skill +
        /// CLAUDE.md + permissions, no hook). Required when running
        /// from a non-TTY (CI, piped input).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Operate on a project at this path instead of the cwd.
        #[arg(long, value_name = "PATH")]
        project_root: Option<PathBuf>,
    },

    /// Hidden — invoked by Claude Code hooks installed via `ga init --with-hook`.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        subcommand: HookSubcommand,
    },

    /// Diagnose install + cache health (S-002).
    #[command(long_about = "\
Run five health checks and print ✓ / ✗ per line with a remediation hint:
  1. Binary in PATH
  2. MCP config valid JSON
  3. graphatlas entry present in the config
  4. Cache dir (~/.graphatlas) writable
  5. Fixture spike repo accessible (dev-only)

Exits 0 if all pass, 1 otherwise. Run before asking for help on GitHub issues.

EXAMPLES:
  graphatlas doctor               # Run all checks.
  GRAPHATLAS_CACHE_DIR=/tmp/g graphatlas doctor
")]
    Doctor,

    /// Wire MCP config into an LLM client (S-002).
    #[command(long_about = "\
Add a `graphatlas` entry to the client's MCP config so the LLM will launch
`graphatlas mcp` as a subprocess. Preserves existing mcpServers entries and
writes a .bak backup before editing.

SUPPORTED --client VALUES: claude, cursor, cline

EXAMPLES:
  graphatlas install --client claude
  graphatlas install --client cursor --config-path ~/.cursor/mcp.json
")]
    Install {
        #[arg(long)]
        client: Option<String>,
        #[arg(long, value_name = "PATH")]
        config_path: Option<PathBuf>,
        /// v1.5 PR7 — install/uninstall/verify the PostToolUse hook for an
        /// agent (`claude-code`, `cursor`, `codex`). Layered with `--client`:
        /// `--client` wires the MCP server, `--hook` wires the post-edit
        /// trigger that auto-calls `ga_reindex`.
        #[arg(long)]
        hook: Option<String>,
        /// Project root for hooks that live under the repo (claude-code,
        /// cursor). Defaults to cwd. Ignored for codex (user-global).
        #[arg(long, value_name = "PATH")]
        project_root: Option<PathBuf>,
        /// Remove the GA hook entry instead of adding it.
        #[arg(long)]
        uninstall: bool,
        /// Report whether the hook is installed correctly. Exits non-zero
        /// on mismatch with an actionable hint.
        #[arg(long)]
        verify: bool,
        /// Allow `--hook` to write through symlinked config files. Default
        /// is to refuse symlinks as a defense against attacker-controlled
        /// path redirection.
        #[arg(long)]
        follow_symlinks: bool,
    },

    /// List cached repo indexes (S-003).
    #[command(long_about = "\
Show one row per cached repo under ~/.graphatlas (or $GRAPHATLAS_CACHE_DIR).
Columns: NAME, REPO PATH, SIZE, LAST INDEXED.

EXAMPLES:
  graphatlas list
  GRAPHATLAS_CACHE_DIR=/tmp/ga graphatlas list
")]
    List,

    /// Run a UC benchmark (Benchmarks S-001).
    #[command(long_about = "\
Run a benchmark for a specific use case (callers, callees, importers,
symbols, file_summary, impact). Produces a Markdown leaderboard comparing
graphatlas against baselines (CGC, CM, CRG, ripgrep).

EXAMPLES:
  graphatlas bench --uc callers
  graphatlas bench --uc impact
")]
    Bench {
        #[arg(long)]
        uc: Option<String>,
        /// Fixture directory name under `benches/fixtures/` (default: `mini`).
        #[arg(long, default_value = "mini")]
        fixture: String,
        /// Comma-separated retriever list. Default = all registered
        /// (`ga,ripgrep,codegraphcontext,codebase-memory`). External tools
        /// disable gracefully if not installed.
        #[arg(long)]
        retrievers: Option<String>,
        /// Gate to dispatch (`m1` default, `m3` for V1.1 decision-support tools).
        /// `m2` runs through the standalone test harness — wiring follow-up.
        #[arg(long)]
        gate: Option<String>,
        /// Regenerate `benches/uc-<uc>/<fixture>.generated.json` from AST-level
        /// auto-GT rules (H1-text, H5-reexport). Does NOT run the bench —
        /// use a second invocation to score.
        #[arg(long)]
        refresh_gt: bool,
        /// When regenerating GT, INCLUDE test-file call sites (default is to
        /// exclude paths like `tests/`, `*_test.py`, `*.spec.ts`). Tests
        /// typically dominate caller expected lists and crowd out the
        /// production signal we're trying to measure.
        #[arg(long)]
        include_tests: bool,
    },

    /// Print manual-download instructions (self-update deferred to v1.1).
    #[command(long_about = "\
Self-update is deferred to v1.1 per Foundation R29/C-5 (TOFU attack concern).
This command prints manual download instructions. To actually upgrade, run
install.sh from GitHub Releases again.

EXAMPLES:
  graphatlas update               # Prints instructions, exits 0.
")]
    Update,

    /// Cache management (S-003).
    #[command(long_about = "\
Manage the per-repo cache at ~/.graphatlas. Future subcommands: clear,
prune, compact. Currently a stub.

EXAMPLES:
  graphatlas cache                # Stub: prints intent and exits.
")]
    Cache,
}

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
            Ok(())
        }
        Some(Command::Update) => {
            println!(
                "Self-update deferred to v1.1. Download latest release:\n  \
                 https://github.com/graphatlas-dev/graphatlas/releases\n\
                 Or reinstall via install.sh from GitHub Releases."
            );
            Ok(())
        }
        Some(Command::Bench {
            uc,
            fixture,
            retrievers,
            gate,
            refresh_gt,
            include_tests,
        }) => graphatlas::bench_cmd::cmd_bench(
            uc,
            fixture,
            retrievers,
            gate,
            refresh_gt,
            include_tests,
        ),
        Some(Command::List) => cmd_list(resolve_cache_root()?),
        Some(Command::Install {
            client,
            config_path,
            hook,
            project_root,
            uninstall,
            verify,
            follow_symlinks,
        }) => cmd_install(
            client,
            config_path,
            hook,
            project_root,
            uninstall,
            verify,
            follow_symlinks,
        ),
        Some(Command::Doctor) => cmd_doctor(),
        Some(Command::Mcp) => cmd_mcp(),
        Some(Command::Init {
            with_skill,
            with_claudemd,
            with_permissions,
            with_hook,
            all,
            remove_hook,
            yes,
            project_root,
        }) => graphatlas::cmd_init::cmd_init(graphatlas::cmd_init::InitOptions {
            project_root,
            with_skill,
            with_claudemd,
            with_permissions,
            with_hook,
            all,
            remove_hook,
            yes,
            binary_path: None,
        }),
        Some(Command::Hook { subcommand }) => match subcommand {
            HookSubcommand::SessionStart => graphatlas::cmd_hook::cmd_hook_session_start(),
        },
        Some(Command::Cache) => {
            println!("graphatlas cache: S-001 stub — not implemented.");
            println!("This subcommand is reserved per Foundation-C6 (8-subcommand lock).");
            println!("Implementation lands in its owning story; see docs/specs/graphatlas-v1/.");
            Ok(())
        }
    }
}

fn cmd_mcp() -> Result<()> {
    let cache_root = resolve_cache_root()?;
    graphatlas::mcp_cmd::cmd_mcp(&cache_root)
}

fn cmd_doctor() -> Result<()> {
    use graphatlas::doctor::{run_doctor, CheckStatus, DoctorOptions};
    let opts = DoctorOptions {
        binary_path: std::env::current_exe().ok(),
        mcp_config_path: resolve_default_mcp_config(),
        cache_root: resolve_cache_root().ok(),
    };
    let report = run_doctor(&opts);
    for check in &report.checks {
        let glyph = match check.status {
            CheckStatus::Ok => "✓",
            CheckStatus::Fail => "✗",
        };
        println!("{glyph} {} — {}", check.name, check.message);
        if let Some(r) = &check.remediation {
            println!("    hint: {r}");
        }
    }
    if report.all_ok() {
        println!("\nAll checks passed.");
    } else {
        println!("\nSome checks failed. See hints above.");
    }
    std::process::exit(report.exit_code());
}

fn resolve_default_mcp_config() -> Option<PathBuf> {
    // Default to Claude's config path. `doctor` is advisory; users running
    // cursor/cline can still wire their own check via env var override
    // (v1.1 can add --client flag).
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude/mcp.json"))
}

fn resolve_cache_root() -> Result<PathBuf> {
    if let Ok(d) = std::env::var("GRAPHATLAS_CACHE_DIR") {
        return Ok(PathBuf::from(d));
    }
    let home = std::env::var("HOME").map_err(|_| {
        anyhow::anyhow!(
            "HOME env var not set; cannot resolve ~/.graphatlas \
             (set GRAPHATLAS_CACHE_DIR to override)"
        )
    })?;
    Ok(PathBuf::from(home).join(".graphatlas"))
}

fn cmd_list(cache_root: PathBuf) -> Result<()> {
    let entries = ga_index::list::list_caches(&cache_root)?;
    if entries.is_empty() {
        println!("(no caches under {})", cache_root.display());
        return Ok(());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name_w = entries
        .iter()
        .map(|e| e.dir_name.len())
        .max()
        .unwrap_or(4)
        .max(24);
    let path_w = entries
        .iter()
        .map(|e| e.repo_root.len())
        .max()
        .unwrap_or(4)
        .max(30);
    let header_name = "NAME";
    let header_path = "REPO PATH";
    let header_size = "SIZE";
    let header_last = "LAST INDEXED";
    println!("{header_name:<name_w$}  {header_path:<path_w$}  {header_size:>6}  {header_last}");
    for e in entries {
        let size = ga_index::list::format_size(e.size_bytes);
        let age = ga_index::list::format_age(e.last_indexed_unix, now);
        let dir_name = e.dir_name;
        let repo_root = e.repo_root;
        println!("{dir_name:<name_w$}  {repo_root:<path_w$}  {size:>6}  {age}");
    }
    Ok(())
}

/// v1.5 PR3 foundation S-004 AS-012 — install a tracing subscriber so
/// downstream calls to `tracing::{info, warn, error}` and `info_span!` are
/// captured.
///
/// Behavior:
/// - **Default** (no `RUST_LOG` env): subscriber is a no-op — tracing events
///   are dropped at near-zero cost. Existing `eprintln!` spec-literal lines
///   (AS-008/AS-027/AS-025 from v1) keep emitting on stderr verbatim so the
///   bench/eval pipeline greppers stay green.
/// - **`RUST_LOG=info` (or `=debug`/etc)**: subscriber installs with the
///   matching filter and writes to stderr.
/// - **`GA_LOG_FORMAT=json`**: subscriber switches to JSON line format
///   (planned for PR4+ once correlation_ids flow through structured logs).
///   For PR3 we accept the env var but JSON formatting is reserved.
///
/// Installation failures are logged to stderr (best-effort) but do NOT
/// abort the process — the user invoked `graphatlas mcp` to serve queries,
/// not to debug logging.
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    // Only install when explicit opt-in via RUST_LOG. Otherwise tracing
    // events are silently discarded — preserves existing eprintln stderr
    // contract for bench tests.
    let Ok(filter_str) = std::env::var("RUST_LOG") else {
        return;
    };
    let filter = match EnvFilter::try_new(&filter_str) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("warn: invalid RUST_LOG '{filter_str}': {e}; tracing disabled");
            return;
        }
    };
    let subscriber = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(false);
    if let Err(e) = subscriber.try_init() {
        eprintln!("warn: tracing subscriber already installed: {e}");
    }
}
