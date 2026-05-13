//! Opt-in JSONL telemetry for dogfood / pre-v1 self-review.
//!
//! Activated by `GRAPHATLAS_TRACE=1`. When off, [`Telemetry::global`]
//! returns `None` and every call site short-circuits — zero overhead,
//! zero risk to the MCP transport.
//!
//! Hard rules (do not relax without re-reading these):
//!   1. Never write to stdout — that is the rmcp JSON-RPC transport.
//!      All output goes to a file under `$GRAPHATLAS_TRACE_DIR` (default
//!      `~/.graphatlas/logs/`). Boot/init failures fall back to stderr.
//!   2. Telemetry MUST NOT alter the dispatch result. Call sites log
//!      *after* the response is built, on a clone of the payload.
//!   3. Every fallible op swallows its error (`let _ = …`). A failing
//!      writer must never propagate into the MCP response path.
//!
//! Schema is JSONL — one event per line. Two event kinds today:
//!   - `boot`: emitted once at server start with environment snapshot.
//!   - `call`: emitted per `tools/call` with input, output, duration, error.
//!
//! Bumping the schema: add fields freely (consumers should ignore unknown);
//! never remove or rename — append-only.
//!
//! The session id correlates events across one server lifetime; `seq`
//! gives stable ordering even if the OS clock jumps.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

static GLOBAL: OnceLock<Option<Telemetry>> = OnceLock::new();

pub struct Telemetry {
    writer: Mutex<BufWriter<File>>,
    session_id: String,
    seq: AtomicU64,
    env: Value,
}

#[derive(Serialize)]
struct CallEvent<'a> {
    ts: String,
    session_id: &'a str,
    seq: u64,
    event: &'static str,
    tool: &'a str,
    input: &'a Value,
    output_chars: usize,
    output: &'a str,
    is_error: bool,
    error: Option<&'a str>,
    duration_ms: u128,
    env: &'a Value,
}

#[derive(Serialize)]
struct BootEvent<'a> {
    ts: String,
    session_id: &'a str,
    seq: u64,
    event: &'static str,
    env: &'a Value,
    extra: Value,
}

impl Telemetry {
    /// Install the global telemetry sink if `GRAPHATLAS_TRACE=1`.
    /// Idempotent — second calls are no-ops.
    pub fn install_global() {
        let _ = GLOBAL.get_or_init(Self::try_init);
    }

    /// Returns the global sink, or `None` if telemetry is disabled or
    /// init failed. Hot-path callers should bail early on `None`.
    pub fn global() -> Option<&'static Telemetry> {
        GLOBAL.get().and_then(|opt| opt.as_ref())
    }

    fn try_init() -> Option<Self> {
        if std::env::var("GRAPHATLAS_TRACE").ok().as_deref() != Some("1") {
            return None;
        }

        let dir = trace_dir();
        if let Err(e) = create_dir_secure(&dir) {
            eprintln!("graphatlas telemetry: mkdir {} failed: {e}", dir.display());
            return None;
        }
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        let path = dir.join(format!("mcp-{day}.jsonl"));
        let file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("graphatlas telemetry: open {} failed: {e}", path.display());
                return None;
            }
        };

        let session_id = new_session_id();
        let env = serde_json::json!({
            "ga_version": env!("CARGO_PKG_VERSION"),
            "mcp_protocol": crate::MCP_PROTOCOL_VERSION,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "pid": std::process::id(),
        });

        // Process-wide panic hook: best-effort flush of last buffered
        // events before the process dies. Chained on top of any pre-existing
        // hook so we don't clobber the user's panic handler.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(t) = Telemetry::global() {
                let _ = t.log_raw(serde_json::json!({
                    "ts": now_iso(),
                    "session_id": t.session_id,
                    "seq": t.next_seq(),
                    "event": "panic",
                    "message": info.to_string(),
                }));
            }
            prev(info);
        }));

        Some(Self {
            writer: Mutex::new(BufWriter::new(file)),
            session_id,
            seq: AtomicU64::new(0),
            env,
        })
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    pub fn log_boot(&self, repo_root: &Path, extra: Value) {
        let env = merge_env(&self.env, repo_root);
        let evt = BootEvent {
            ts: now_iso(),
            session_id: &self.session_id,
            seq: self.next_seq(),
            event: "boot",
            env: &env,
            extra,
        };
        let _ = self.log_serializable(&evt);
    }

    pub fn log_call(
        &self,
        tool: &str,
        input: &Value,
        output: &str,
        is_error: bool,
        error: Option<&str>,
        duration: Duration,
    ) {
        let evt = CallEvent {
            ts: now_iso(),
            session_id: &self.session_id,
            seq: self.next_seq(),
            event: "call",
            tool,
            input,
            output_chars: output.chars().count(),
            output,
            is_error,
            error,
            duration_ms: duration.as_millis(),
            env: &self.env,
        };
        let _ = self.log_serializable(&evt);
    }

    fn log_serializable<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        let line = serde_json::to_string(value).map_err(std::io::Error::other)?;
        let mut w = match self.writer.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        writeln!(w, "{line}")?;
        w.flush()?;
        Ok(())
    }

    fn log_raw(&self, value: Value) -> std::io::Result<()> {
        self.log_serializable(&value)
    }
}

/// Create the telemetry log dir without violating ga-index's vault
/// invariant: `~/.graphatlas/` MUST be 0700 (see `Store::open_with_root`).
/// Default umask would yield 0755 and corrupt the cache config; we set
/// 0700 explicitly on Unix and walk up from the leaf to fix any parent
/// we own that pre-existed at a looser mode.
fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p: Option<&Path> = Some(dir);
        let home = std::env::var("HOME").unwrap_or_default();
        while let Some(cur) = p {
            // Only touch paths under HOME/.graphatlas to avoid surprising
            // chmod on user-supplied GRAPHATLAS_TRACE_DIR roots.
            if !home.is_empty()
                && cur.starts_with(&home)
                && cur != Path::new(&home)
                && cur.file_name().is_some()
            {
                let _ = std::fs::set_permissions(cur, std::fs::Permissions::from_mode(0o700));
            }
            p = cur.parent();
        }
    }
    Ok(())
}

fn trace_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("GRAPHATLAS_TRACE_DIR") {
        return PathBuf::from(custom);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".graphatlas").join("logs")
}

fn new_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032x}")
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn merge_env(base: &Value, repo_root: &Path) -> Value {
    let mut env = base.clone();
    if let Some(obj) = env.as_object_mut() {
        obj.insert(
            "repo_root".into(),
            Value::String(repo_root.display().to_string()),
        );
        if let Ok(cwd) = std::env::current_dir() {
            obj.insert("cwd".into(), Value::String(cwd.display().to_string()));
        }
        let (sha, dirty) = git_head(repo_root);
        if let Some(s) = sha {
            obj.insert("git_sha".into(), Value::String(s));
        }
        obj.insert("git_dirty".into(), Value::Bool(dirty));
    }
    env
}

fn git_head(repo_root: &Path) -> (Option<String>, bool) {
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);
    (sha, dirty)
}
