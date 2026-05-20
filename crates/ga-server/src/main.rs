//! `ga-server` binary entry point.
//!
//! Spawned by `ga ui` (Spec D S-001 AS-001). All CSRF gates live in
//! `ga_server::middleware::security`; this layer just resolves CLI args
//! and hands them to `ga_server::serve`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

use ga_server::{
    build_app_with_static, validate_bind_addr, AppState, LbugDataSource, ServerConfig,
    SubprocessLauncher,
};

async fn serve_with_optional_static(
    state: AppState,
    addr: std::net::SocketAddr,
    ui_dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    validate_bind_addr(&addr).map_err(anyhow::Error::msg)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {} failed: {} (AS-006 port conflict?)", addr, e))?;
    tracing::info!(target: "ga_server", "listening on http://{}", addr);
    axum::serve(listener, build_app_with_static(state, ui_dir))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!(target: "ga_server", "shutdown signal received");
        })
        .await
        .map_err(Into::into)
}

/// CLI args. Defaults align with Spec D D-Cn invariants:
///   --port 4317           (Spec D Data Model)
///   --bind 127.0.0.1      (Spec A AS-002 — only loopback accepted)
#[derive(Parser, Debug)]
#[command(name = "ga-server", about = "GraphAtlas UI HTTP backend")]
struct Cli {
    /// TCP port to bind on.
    #[arg(long, default_value_t = 4317)]
    port: u16,
    /// Bind address. Phase 1 refuses non-loopback — see AS-002.
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// Cache root for indexed projects (default `~/.graphatlas`).
    #[arg(long)]
    cache_root: Option<PathBuf>,
    /// Per-session token (hex). Spawned by `ga ui` with `--token=<hex>`.
    /// Frontend bootstraps this via URL hash (Spec D AS-001).
    #[arg(long)]
    token: String,
    /// Frontend Bun.serve port (used to build Origin/Host allowlists).
    #[arg(long, default_value_t = 4318)]
    frontend_port: u16,
    /// Optional static-file root served at `/` (Phase 1 stand-in for
    /// the separate Bun.serve frontend — lets `ga ui` orchestrate a
    /// single process until Spec B/C ships a real React bundle).
    #[arg(long)]
    ui_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    // tracing-subscriber default: log INFO+ to stderr. The parent
    // `ga ui` process prefixes lines with `[server]` for grep clarity
    // (Spec D D-C4).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // AS-002 — defense-in-depth: reject non-loopback at CLI before we
    // even hit the library validator. Exit code 2 documents config
    // misuse vs. exit 1 = runtime failure.
    let ip: std::net::IpAddr = match cli.bind.parse() {
        Ok(ip) => ip,
        Err(e) => {
            eprintln!("ga-server: invalid --bind {:?}: {}", cli.bind, e);
            return ExitCode::from(2);
        }
    };
    if !ip.is_loopback() {
        eprintln!(
            "ga-server: Phase 1 bind 127.0.0.1 only; got {} (per Spec A AS-002 / C-cross-2)",
            ip
        );
        return ExitCode::from(2);
    }

    // Token check retired 2026-05-17 — middleware no longer reads
    // `cfg.token`. Accept any value (including empty); the CLI flag
    // stays for back-compat with launch scripts that still pass it.
    let _ = cli.token.len();

    let cache_root = cli
        .cache_root
        .or_else(|| std::env::var_os("GRAPHATLAS_CACHE_DIR").map(PathBuf::from))
        .unwrap_or_else(|| {
            dirs_home().unwrap_or_else(|| PathBuf::from(".")).join(".graphatlas")
        });

    let addr = SocketAddr::new(ip, cli.port);
    let cfg = ServerConfig {
        bind: addr,
        cache_root,
        token: cli.token,
        allowed_origins: ServerConfig::origins_for_single_process(cli.port, cli.frontend_port),
        allowed_hosts: ServerConfig::hosts_for_port(cli.port),
        frontend_origin: format!("http://localhost:{}", cli.frontend_port),
    };
    let launcher: Arc<dyn ga_server::JobLauncher> = match SubprocessLauncher::from_current_exe() {
        Ok(l) => Arc::new(l),
        Err(e) => {
            eprintln!("ga-server: cannot resolve graphatlas binary path: {}", e);
            return ExitCode::from(1);
        }
    };
    let data: Arc<dyn ga_server::ProjectDataSource> =
        Arc::new(LbugDataSource::new(cfg.cache_root.clone()));
    let watcher_registry = Arc::new(ga_server::watcher::WatcherRegistry::new());
    let watcher_driver: Arc<dyn ga_server::watcher::WatcherDriver> = Arc::new(
        ga_server::watcher::NotifyWatcherDriver::new(watcher_registry.clone()),
    );
    let mut state = AppState::new(cfg, launcher, data, watcher_driver);
    state.watchers = watcher_registry;
    let state = state;
    let ui_dir = cli.ui_dir;

    // Build and run. AS-006 port conflict surfaces as a bind error
    // inside `serve()` — we map it to exit 1.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match rt.block_on(serve_with_optional_static(state, addr, ui_dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ga-server: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Minimal `$HOME` resolver so we avoid pulling the `dirs` crate just
/// for this. Falls back to `.` if neither HOME nor USERPROFILE is set.
fn dirs_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("HOME") {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}
