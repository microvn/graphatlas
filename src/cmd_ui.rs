//! `ga ui` subcommand — Spec D orchestration.
//!
//! Phase 1 smoke build. The orchestration end-to-end (real spawn of
//! ga-server + Bun + browser open) is exercised manually; this module
//! ships the **pure logic** units behind every AS and a thin
//! orchestrator that wires them together.
//!
//! Logic units (all unit-tested in `#[cfg(test)] mod tests`):
//!
//! - `generate_session_token` — 32-byte CSPRNG hex token (AS-018)
//! - `write_session_file`     — 0600 perm + atomic write (AS-018)
//! - `read_session_file`      — parse session JSON (AS-016/017)
//! - `check_lock_status`      — PID alive probe (AS-016 reject vs AS-017 reclaim)
//! - `validate_bind_str`      — loopback enforcement (AS-008)
//! - `resolve_cache_root`     — env / arg / default (AS-009)
//! - `resolve_frontend_bundle`— --ui-dir → env → sibling → workspace → install (AS-013/014)
//! - `check_bun_available`    — PATH probe (AS-012)
//!
//! Manual-smoke AS map (Spec D `.build-checklist`):
//! AS-001/003/004/005/006/007/010/011/015 — real subprocess interaction.

use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

// ---------- CLI args (mirrors Spec D Data Model) ----------

/// What `main.rs` populates from the parsed clap args. Stays a plain
/// struct so the tests can construct it directly without a clap parse.
#[derive(Debug, Clone)]
pub struct UiArgs {
    pub port: u16,
    pub frontend_port: u16,
    pub bind: String,
    pub no_open: bool,
    pub cache_root: Option<PathBuf>,
    pub log_level: String,
    pub dev: bool,
    pub ui_dir: Option<PathBuf>,
}

impl Default for UiArgs {
    fn default() -> Self {
        Self {
            port: 4317,
            frontend_port: 4318,
            bind: "127.0.0.1".into(),
            no_open: false,
            cache_root: None,
            log_level: "info".into(),
            dev: false,
            ui_dir: None,
        }
    }
}

// ---------- Session file (`.ui.session`) — AS-016/017/018 ----------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SessionLock {
    pub token: String,
    pub port: u16,
    pub pid: u32,
    pub started_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LockStatus {
    /// No lock file present — free to start.
    Free,
    /// Existing lock file, recorded PID still alive — refuse start.
    OwnedAlive(SessionLock),
    /// Existing lock file but PID dead — reclaim.
    OwnedStale(SessionLock),
}

/// Path to `~/.graphatlas/.ui.session`. Caller supplies cache root so
/// tests can use tempdirs.
pub fn session_file_path(cache_root: &Path) -> PathBuf {
    cache_root.join(".ui.session")
}

/// Generate a 32-byte cryptographic token rendered as 64 hex chars.
///
/// Uses `/dev/urandom` on Unix. Windows path falls back to a system-
/// time + thread-id mix — documented limitation for Phase 1 (single
/// dogfood user on macOS/Linux); revisit before non-loopback bind ever
/// becomes an option (Phase 2).
pub fn generate_session_token() -> Result<String> {
    let mut buf = [0u8; 32];
    fill_random(&mut buf)?;
    Ok(hex::encode(buf))
}

#[cfg(unix)]
fn fill_random(out: &mut [u8]) -> Result<()> {
    let mut f = std::fs::File::open("/dev/urandom")
        .context("opening /dev/urandom — Phase 1 Unix-only entropy source")?;
    f.read_exact(out).context("reading /dev/urandom")?;
    Ok(())
}

#[cfg(not(unix))]
fn fill_random(out: &mut [u8]) -> Result<()> {
    // Phase 1 fallback — RandomState + nanoseconds. Acceptable for
    // local 127.0.0.1 binding; revisit before any Phase 2 remote bind.
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    for chunk in out.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        );
        let n = h.finish().to_le_bytes();
        let len = chunk.len();
        chunk.copy_from_slice(&n[..len]);
    }
    Ok(())
}

/// Write `.ui.session` with 0600 perms (atomic via tmp + rename).
/// AS-018 — overwriting an existing file invalidates the prior token.
pub fn write_session_file(cache_root: &Path, lock: &SessionLock) -> Result<()> {
    std::fs::create_dir_all(cache_root).ok();
    let path = session_file_path(cache_root);
    let tmp = path.with_extension("session.tmp");
    let body = serde_json::to_vec_pretty(lock).context("serialize session lock")?;
    std::fs::write(&tmp, body).context("write tmp session file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&tmp, perm).context("chmod 0600 tmp session file")?;
    }
    std::fs::rename(&tmp, &path).context("atomic rename session file")?;
    Ok(())
}

/// Parse `.ui.session`. Returns `Ok(None)` if file missing; error on
/// malformed JSON so callers can decide whether to reclaim or refuse.
pub fn read_session_file(cache_root: &Path) -> Result<Option<SessionLock>> {
    let path = session_file_path(cache_root);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow!("read session file: {e}")),
    };
    let lock: SessionLock = serde_json::from_slice(&bytes).context("parse .ui.session")?;
    Ok(Some(lock))
}

/// Decide what to do about a `.ui.session` file. AS-016 (reject alive
/// peer) vs AS-017 (reclaim stale lock).
pub fn check_lock_status(cache_root: &Path, pid_alive: impl Fn(u32) -> bool) -> Result<LockStatus> {
    let Some(lock) = read_session_file(cache_root)? else {
        return Ok(LockStatus::Free);
    };
    if pid_alive(lock.pid) {
        Ok(LockStatus::OwnedAlive(lock))
    } else {
        Ok(LockStatus::OwnedStale(lock))
    }
}

/// Default PID-alive probe. Unix: shells out to `kill -0 PID` (no
/// signal sent — just permission/existence check). Workspace forbids
/// `unsafe` so we can't call `libc::kill` directly; the subprocess is
/// cheap (single syscall fork/exec) and only runs once per `ga ui`
/// start. On non-Unix Phase 1, conservative: treat as alive (refuse).
pub fn default_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

// ---------- bind validation — AS-008 ----------

pub fn validate_bind_str(s: &str) -> Result<IpAddr> {
    let ip: IpAddr = s.parse().with_context(|| format!("invalid --bind {s:?}"))?;
    if !ip.is_loopback() {
        return Err(anyhow!(
            "Phase 1: --bind 127.0.0.1 only; got {ip} (per Spec D AS-008)"
        ));
    }
    Ok(ip)
}

// ---------- cache root — AS-009 ----------

/// Resolve the cache root following the same precedence as the rest of
/// the binary: explicit arg → `GRAPHATLAS_CACHE_DIR` env → `~/.graphatlas`.
pub fn resolve_cache_root(arg: Option<&Path>, env: Option<&Path>, home: &Path) -> PathBuf {
    if let Some(p) = arg {
        return p.to_path_buf();
    }
    if let Some(p) = env {
        return p.to_path_buf();
    }
    home.join(".graphatlas")
}

// ---------- frontend bundle resolution — AS-013/014 ----------

#[derive(Debug, Clone)]
pub struct BundleResolveArgs<'a> {
    /// `--ui-dir` flag (highest precedence).
    pub flag: Option<&'a Path>,
    /// `GA_UI_DIR` env.
    pub env: Option<&'a Path>,
    /// `<ga binary dir>` for sibling lookup.
    pub ga_bin_dir: Option<&'a Path>,
    /// Workspace root (dev checkout); resolver checks
    /// `<root>/dist/ui/` (bundled output of `bun run build`).
    /// Source `<root>/ui/` is NOT a valid bundle — it holds raw `.tsx`
    /// which the browser can't execute. `run()` auto-bundles when only
    /// the source dir exists.
    pub workspace_root: Option<&'a Path>,
    /// System-wide install fallback (typically `~/.graphatlas`).
    pub install_fallback: Option<&'a Path>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BundleResolutionError {
    /// All candidate paths checked, none exists. Includes the list for
    /// the error message — AS-013 requires showing what was attempted.
    NotFound(Vec<PathBuf>),
}

/// Resolve in the order documented in Spec D Data Model:
///   1. `--ui-dir`
///   2. `GA_UI_DIR`
///   3. `<ga bin dir>/../ui-bundle/`
///   4. `<workspace root>/dist/ui/`  (bundled — `bun run build` output)
///   5. `<install fallback>/ui-bundle/` (typically `~/.graphatlas/ui-bundle/`)
///
/// Returns the first existing directory. If none exist, returns
/// `NotFound` listing every candidate so the CLI error can surface
/// what was tried (AS-013).
pub fn resolve_frontend_bundle(
    args: &BundleResolveArgs<'_>,
) -> std::result::Result<PathBuf, BundleResolutionError> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(p) = args.flag {
        candidates.push(p.to_path_buf());
    }
    if let Some(p) = args.env {
        candidates.push(p.to_path_buf());
    }
    if let Some(d) = args.ga_bin_dir {
        candidates.push(d.join("..").join("ui-bundle"));
    }
    if let Some(d) = args.workspace_root {
        candidates.push(d.join("dist").join("ui"));
    }
    if let Some(d) = args.install_fallback {
        candidates.push(d.join("ui-bundle"));
    }

    for cand in &candidates {
        if cand.is_dir() {
            return Ok(cand.clone());
        }
    }
    Err(BundleResolutionError::NotFound(candidates))
}

// ---------- auto-bundle fallback (dev workspace UX) ----------

/// Run `bun build ui/index.html --outdir <target> --minify` from
/// `workspace_root`. Spec D follow-up: zero-friction first `ga ui`
/// invocation when only the source `ui/` exists. Subsequent runs
/// reuse the bundle.
///
/// Failure surfaces as anyhow Error — caller adds context pointing
/// at the manual escape hatch (`bun run build`).
fn auto_bundle(bun_bin: &Path, workspace_root: &Path, target: &Path) -> Result<()> {
    eprintln!(
        "ga ui: bundling frontend → {} (first run; ~1s)",
        target.display()
    );
    let status = std::process::Command::new(bun_bin)
        .current_dir(workspace_root)
        .args([
            "build",
            "ui/index.html",
            "--outdir",
            target
                .to_str()
                .ok_or_else(|| anyhow!("non-utf8 target path"))?,
            "--minify",
        ])
        .status()
        .with_context(|| format!("spawning `{} build`", bun_bin.display()))?;
    if !status.success() {
        return Err(anyhow!("bun build exited with {status}"));
    }
    if !target.join("index.html").is_file() {
        return Err(anyhow!(
            "bun build reported success but {} missing",
            target.join("index.html").display()
        ));
    }
    Ok(())
}

// ---------- bun availability — AS-012 ----------

/// Verify `bun` resolves in PATH. Returns the absolute path so the
/// orchestrator can spawn with `Command::new(absolute)` rather than
/// trusting PATH at exec time.
pub fn check_bun_available(path_env: Option<&str>) -> Result<PathBuf> {
    let path = path_env.unwrap_or("");
    for dir in std::env::split_paths(path) {
        let candidate = dir.join("bun");
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&candidate) {
                    if meta.permissions().mode() & 0o111 != 0 {
                        return Ok(candidate);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                return Ok(candidate);
            }
        }
    }
    Err(anyhow!(
        "Bun runtime required — install via `curl -fsSL https://bun.sh/install | bash`"
    ))
}

// ---------- Orchestration entry point — AS-001 (manual smoke) ----------

/// Thin orchestrator. Wires the pure-logic units together; the real
/// subprocess spawns + browser open happen here.
///
/// Returns `Ok(())` on graceful Ctrl+C, non-zero anyhow Error on
/// failure. The orchestration is **not** unit-tested — its component
/// pieces are. See `.build-checklist` for manual-smoke AS map.
pub fn run(args: UiArgs) -> Result<()> {
    let ip = validate_bind_str(&args.bind)?;
    let home = dirs_home().ok_or_else(|| anyhow!("could not resolve $HOME for cache root"))?;
    let env_cache = std::env::var_os("GRAPHATLAS_CACHE_DIR").map(PathBuf::from);
    let cache_root = resolve_cache_root(args.cache_root.as_deref(), env_cache.as_deref(), &home);

    // AS-016/017 — single-instance lock.
    match check_lock_status(&cache_root, default_pid_alive)? {
        LockStatus::OwnedAlive(lock) => {
            return Err(anyhow!(
                "Another ga ui running on :{} (pid {}); only 1 instance per machine Phase 1",
                lock.port,
                lock.pid
            ));
        }
        LockStatus::OwnedStale(_) | LockStatus::Free => {
            // Free or reclaimable — proceed.
        }
    }

    // Lock file: port + pid for single-instance enforcement (AS-016/017).
    // Token field retained as empty string for back-compat with parsing;
    // server no longer requires X-GA-Token (auth removed 2026-05-17).
    let lock = SessionLock {
        token: String::new(),
        port: args.port,
        pid: std::process::id(),
        started_at: now_unix(),
    };
    write_session_file(&cache_root, &lock)?;

    // AS-012 — verify bun is available before we spawn the backend.
    let bun_bin = check_bun_available(std::env::var("PATH").ok().as_deref())?;

    // AS-013/014 — bundle resolution.
    let ga_bin = std::env::current_exe().ok();
    let ga_bin_dir = ga_bin.as_deref().and_then(|p| p.parent());
    let env_ui = std::env::var_os("GA_UI_DIR").map(PathBuf::from);
    let workspace = workspace_root_guess();
    let install_fallback = cache_root.clone();
    let bundle_dir = match resolve_frontend_bundle(&BundleResolveArgs {
        flag: args.ui_dir.as_deref(),
        env: env_ui.as_deref(),
        ga_bin_dir,
        workspace_root: workspace.as_deref(),
        install_fallback: Some(&install_fallback),
    }) {
        Ok(d) => d,
        Err(BundleResolutionError::NotFound(paths)) => {
            // Auto-bundle fallback (option B): if we're in a dev
            // workspace where `ui/index.html` exists but `dist/ui/`
            // doesn't yet, run `bun build` once to produce the bundle.
            // First `ga ui` invocation is zero-friction; subsequent
            // invocations hit the fast path because dist/ui exists.
            if let Some(ws) = workspace.as_deref() {
                let src_html = ws.join("ui").join("index.html");
                let target = ws.join("dist").join("ui");
                if src_html.is_file() {
                    auto_bundle(&bun_bin, ws, &target).with_context(|| {
                        format!(
                            "auto-bundle failed; run `bun run build` manually in {}",
                            ws.display()
                        )
                    })?;
                    target
                } else {
                    return Err(anyhow!(
                        "Frontend bundle not found at any of:\n  {}\nset GA_UI_DIR or --ui-dir",
                        paths
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join("\n  ")
                    ));
                }
            } else {
                return Err(anyhow!(
                    "Frontend bundle not found at any of:\n  {}\nset GA_UI_DIR or --ui-dir",
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                ));
            }
        }
    };

    // D-S001 follow-up — real ga-server spawn + health probe + browser
    // open + signal-blocking. Phase 1 ships single-process orchestration
    // (ga-server serves both /api and static `<bundle>` at /) so we
    // don't depend on a separate Bun.serve process until the real
    // React frontend lands. `bun` is still verified for forward-compat.
    let _ = bun_bin;

    let ga_server_bin = resolve_ga_server_bin(&args)?;
    eprintln!("ga ui starting:");
    eprintln!("  backend bin : {}", ga_server_bin.display());
    eprintln!("  ui bundle   : {}", bundle_dir.display());
    eprintln!("  cache root  : {}", cache_root.display());

    let mut child = std::process::Command::new(&ga_server_bin)
        .arg("--port")
        .arg(args.port.to_string())
        .arg("--bind")
        .arg(format!("{}", ip))
        // --token retained for ga-server CLI back-compat; empty value
        // means "auth disabled" (the middleware never reads it anyway).
        .arg("--token")
        .arg("")
        .arg("--frontend-port")
        .arg(args.frontend_port.to_string())
        .arg("--cache-root")
        .arg(&cache_root)
        .arg("--ui-dir")
        .arg(&bundle_dir)
        .spawn()
        .with_context(|| format!("spawn {}", ga_server_bin.display()))?;

    // AS-011 — health probe with 15s timeout. Port-open is sufficient
    // (full /api/health round-trip needs a token; we just want to know
    // the listener bound).
    let probe_addr = std::net::SocketAddr::new(ip, args.port);
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(15);
    loop {
        if std::net::TcpStream::connect_timeout(&probe_addr, std::time::Duration::from_millis(200))
            .is_ok()
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = std::fs::remove_file(session_file_path(&cache_root));
            return Err(anyhow!(
                "ga-server not ready after 15s — check logs (Spec D AS-011)"
            ));
        }
        // Detect early child exit (port-conflict, panic).
        if let Ok(Some(status)) = child.try_wait() {
            let _ = std::fs::remove_file(session_file_path(&cache_root));
            return Err(anyhow!(
                "ga-server exited before becoming ready (status {:?}); check stderr",
                status.code()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    let url = format!("http://{}:{}/", ip, args.port);
    eprintln!("\nga ui ready:");
    eprintln!("  open {url}");
    eprintln!("  Ctrl+C to stop\n");

    // AS-002 / AS-010 — open browser unless suppressed or headless.
    if !args.no_open {
        if let Err(e) = open_browser(&url) {
            eprintln!("(could not auto-open browser: {e}; visit the URL above)");
        }
    }

    // AS-005 — block until ga-server exits or Ctrl+C. SIGINT goes to
    // both this process AND the child (shared process group), so we
    // just wait on the child here. Our parent receiving SIGINT will
    // ALSO try to wait — that's the same waitpid call returning the
    // child's exit status. Clean.
    let status = child.wait().context("wait ga-server child")?;
    let _ = std::fs::remove_file(session_file_path(&cache_root));
    if !status.success() {
        eprintln!("ga-server exited with status {:?}", status.code());
    }
    Ok(())
}

/// Resolve the path to the `ga-server` binary. Search order:
///   1. `<dir of current_exe>/ga-server` (release tarball sibling)
///   2. `<dir of current_exe>/../ga-server` (cargo workspace layout —
///      target/debug/ga-server is sibling to target/debug/graphatlas)
///   3. `ga-server` on PATH (fallback)
fn resolve_ga_server_bin(_args: &UiArgs) -> Result<PathBuf> {
    let me = std::env::current_exe().context("current_exe")?;
    if let Some(parent) = me.parent() {
        let sibling = parent.join("ga-server");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    // PATH fallback.
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("ga-server");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(anyhow!(
        "ga-server binary not found; ensure `cargo build -p ga-server` ran or install it on PATH"
    ))
}

/// Cross-platform browser launcher. Shells out via the OS-native
/// opener; returns an error if the opener itself fails or isn't found.
/// AS-010 — headless environments (no DISPLAY) typically have `open` /
/// `xdg-open` fail; the caller logs + continues.
fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "windows")]
    let opener = "rundll32";

    #[cfg(not(target_os = "windows"))]
    {
        let status = std::process::Command::new(opener)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .with_context(|| format!("spawn {opener}"))?;
        if !status.success() {
            return Err(anyhow!("{opener} exited non-zero"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new(opener)
            .args(["url.dll,FileProtocolHandler", url])
            .status()
            .context("spawn rundll32")?;
        if !status.success() {
            return Err(anyhow!("rundll32 exited non-zero"));
        }
    }
    Ok(())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn workspace_root_guess() -> Option<PathBuf> {
    // The graphatlas binary inside a cargo target lives at
    // `<workspace>/target/<profile>/graphatlas` — walk up.
    let exe = std::env::current_exe().ok()?;
    let mut p = exe.parent()?.parent()?.parent()?.to_path_buf();
    if p.join("Cargo.toml").is_file() {
        return Some(p);
    }
    // Otherwise climb one more (workspace with virtual manifest +
    // members nesting).
    p = p.parent()?.to_path_buf();
    if p.join("Cargo.toml").is_file() {
        Some(p)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ---------- AS-018 token + session file ----------

    #[test]
    fn as018_token_is_64_hex_chars() {
        let t = generate_session_token().unwrap();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn as018_token_rotation_distinct() {
        let a = generate_session_token().unwrap();
        let b = generate_session_token().unwrap();
        assert_ne!(a, b, "subsequent tokens must differ (CSPRNG)");
    }

    #[test]
    fn as018_session_file_is_atomic_and_0600() {
        let cache = tempdir().unwrap();
        let lock = SessionLock {
            token: "deadbeef".repeat(8),
            port: 4317,
            pid: 12345,
            started_at: 1,
        };
        write_session_file(cache.path(), &lock).unwrap();
        let path = session_file_path(cache.path());
        assert!(path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "session file must be 0600");
        }
        let parsed = read_session_file(cache.path()).unwrap().unwrap();
        assert_eq!(parsed.token, lock.token);
        assert_eq!(parsed.port, lock.port);
    }

    #[test]
    fn as018_write_overwrites_prior_token() {
        let cache = tempdir().unwrap();
        let lock1 = SessionLock {
            token: "a".repeat(64),
            port: 1,
            pid: 1,
            started_at: 1,
        };
        let lock2 = SessionLock {
            token: "b".repeat(64),
            port: 2,
            pid: 2,
            started_at: 2,
        };
        write_session_file(cache.path(), &lock1).unwrap();
        write_session_file(cache.path(), &lock2).unwrap();
        let parsed = read_session_file(cache.path()).unwrap().unwrap();
        assert_eq!(parsed.token, "b".repeat(64));
    }

    // ---------- AS-016 / AS-017 lock status ----------

    #[test]
    fn as016_lock_absent_is_free() {
        let cache = tempdir().unwrap();
        let status = check_lock_status(cache.path(), |_| true).unwrap();
        assert_eq!(status, LockStatus::Free);
    }

    #[test]
    fn as016_lock_alive_is_owned() {
        let cache = tempdir().unwrap();
        let lock = SessionLock {
            token: "x".repeat(64),
            port: 4317,
            pid: 999,
            started_at: 1,
        };
        write_session_file(cache.path(), &lock).unwrap();
        let status = check_lock_status(cache.path(), |_| true).unwrap();
        assert!(matches!(status, LockStatus::OwnedAlive(_)));
    }

    #[test]
    fn as017_lock_dead_pid_reclaimable() {
        let cache = tempdir().unwrap();
        let lock = SessionLock {
            token: "x".repeat(64),
            port: 4317,
            pid: 999,
            started_at: 1,
        };
        write_session_file(cache.path(), &lock).unwrap();
        let status = check_lock_status(cache.path(), |_| false).unwrap();
        assert!(matches!(status, LockStatus::OwnedStale(_)));
    }

    // ---------- AS-008 bind validation ----------

    #[test]
    fn as008_loopback_v4_accepted() {
        assert!(validate_bind_str("127.0.0.1").is_ok());
    }

    #[test]
    fn as008_loopback_v6_accepted() {
        assert!(validate_bind_str("::1").is_ok());
    }

    #[test]
    fn as008_non_loopback_rejected() {
        let err = validate_bind_str("0.0.0.0").unwrap_err();
        assert!(err.to_string().contains("127.0.0.1"));
    }

    #[test]
    fn as008_invalid_str_rejected() {
        assert!(validate_bind_str("not-an-ip").is_err());
    }

    // ---------- AS-009 cache root resolution ----------

    #[test]
    fn as009_arg_wins() {
        let p = resolve_cache_root(
            Some(Path::new("/tmp/from-arg")),
            Some(Path::new("/tmp/from-env")),
            Path::new("/home/x"),
        );
        assert_eq!(p, PathBuf::from("/tmp/from-arg"));
    }

    #[test]
    fn as009_env_falls_through() {
        let p = resolve_cache_root(None, Some(Path::new("/tmp/from-env")), Path::new("/home/x"));
        assert_eq!(p, PathBuf::from("/tmp/from-env"));
    }

    #[test]
    fn as009_home_default() {
        let p = resolve_cache_root(None, None, Path::new("/home/x"));
        assert_eq!(p, PathBuf::from("/home/x/.graphatlas"));
    }

    // ---------- AS-012 bun availability ----------

    #[test]
    fn as012_bun_not_in_path_errs() {
        let empty = tempdir().unwrap();
        let path_str = empty.path().display().to_string();
        let err = check_bun_available(Some(&path_str)).unwrap_err();
        assert!(err.to_string().contains("Bun runtime"));
    }

    #[test]
    #[cfg(unix)]
    fn as012_bun_present_in_path_resolved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let fake_bun = dir.path().join("bun");
        std::fs::write(&fake_bun, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perm = std::fs::metadata(&fake_bun).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&fake_bun, perm).unwrap();
        let path_str = dir.path().display().to_string();
        let resolved = check_bun_available(Some(&path_str)).unwrap();
        assert_eq!(resolved, fake_bun);
    }

    // ---------- AS-013 / AS-014 frontend bundle resolution ----------

    #[test]
    fn as014_flag_wins_over_env() {
        let dir_flag = tempdir().unwrap();
        let dir_env = tempdir().unwrap();
        let args = BundleResolveArgs {
            flag: Some(dir_flag.path()),
            env: Some(dir_env.path()),
            ga_bin_dir: None,
            workspace_root: None,
            install_fallback: None,
        };
        let resolved = resolve_frontend_bundle(&args).unwrap();
        assert_eq!(resolved, dir_flag.path());
    }

    #[test]
    fn as014_env_wins_over_sibling() {
        let dir_env = tempdir().unwrap();
        let sibling_parent = tempdir().unwrap();
        std::fs::create_dir_all(sibling_parent.path().join("../ui-bundle")).ok();
        let args = BundleResolveArgs {
            flag: None,
            env: Some(dir_env.path()),
            ga_bin_dir: Some(sibling_parent.path()),
            workspace_root: None,
            install_fallback: None,
        };
        assert_eq!(resolve_frontend_bundle(&args).unwrap(), dir_env.path());
    }

    #[test]
    fn as014_workspace_ui_fallback() {
        // Workspace fallback now points at `dist/ui` (bundled output of
        // `bun run build`), NOT raw `ui/` source. Source dir has
        // `<script src="./frontend.tsx">` which the browser can't run —
        // see resolve_frontend_bundle doc.
        let workspace = tempdir().unwrap();
        let bundle_dir = workspace.path().join("dist").join("ui");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let args = BundleResolveArgs {
            flag: None,
            env: None,
            ga_bin_dir: None,
            workspace_root: Some(workspace.path()),
            install_fallback: None,
        };
        assert_eq!(resolve_frontend_bundle(&args).unwrap(), bundle_dir);
    }

    #[test]
    fn workspace_ui_source_only_is_rejected() {
        // Reverse: a workspace with ONLY `ui/` source (no `dist/ui`)
        // must NOT resolve to the source dir. Caller's auto-bundle
        // path takes over via the NotFound error.
        let workspace = tempdir().unwrap();
        let src = workspace.path().join("ui");
        std::fs::create_dir_all(&src).unwrap();
        let args = BundleResolveArgs {
            flag: None,
            env: None,
            ga_bin_dir: None,
            workspace_root: Some(workspace.path()),
            install_fallback: None,
        };
        assert!(matches!(
            resolve_frontend_bundle(&args),
            Err(BundleResolutionError::NotFound(_))
        ));
    }

    #[test]
    fn as013_all_missing_returns_not_found_with_list() {
        let args = BundleResolveArgs {
            flag: Some(Path::new("/tmp/nope-flag")),
            env: Some(Path::new("/tmp/nope-env")),
            ga_bin_dir: None,
            workspace_root: None,
            install_fallback: None,
        };
        let err = resolve_frontend_bundle(&args).unwrap_err();
        match err {
            BundleResolutionError::NotFound(paths) => {
                assert!(paths.iter().any(|p| p.ends_with("nope-flag")));
                assert!(paths.iter().any(|p| p.ends_with("nope-env")));
            }
        }
    }

    // ---------- AS-002 / --no-open are arg-parse only ----------
    // The clap derive in main.rs propagates these into UiArgs; the
    // `Default` impl above documents the happy values, and AS-002's
    // behavior ("skip browser open") is consumed inside `run()` which
    // is exercised by manual smoke per Spec D AS-001.
}
