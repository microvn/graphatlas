//! Job tracking + 2-step DELETE token store.
//!
//! `JobRegistry` covers Spec A AS-021 (atomic check-and-insert per slug
//! so concurrent POST for the same path doesn't spawn two `ga reindex`
//! subprocesses). `ConfirmTokens` covers AS-022 / AS-024 (2-step
//! DELETE with 30-second TTL).
//!
//! `JobLauncher` is an injection seam so tests don't have to spawn real
//! subprocesses. The default `SubprocessLauncher` invokes the
//! `graphatlas` binary via argv list (Spec A A-C6 — no shell exec).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

// ============== S-005 — reindex job state machine ==============

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum JobStatus {
    Running,
    Done,
    Error,
    Cancelled,
}

/// Mutable progress + outcome carried by every JobHandle. Wrapped in
/// `Arc<Mutex<>>` so the handler that polls GET /status and the
/// subprocess monitor task (Spec A S-005 follow-up) can share state.
#[derive(Debug, Clone)]
pub struct JobState {
    pub status: JobStatus,
    pub percent: f32,
    /// Name of the indexer phase currently executing (`opening`,
    /// `indexing`, `graph`, `committing`, `done`). Distinct from
    /// `current_file` which is reserved for per-file progress when
    /// the deep `reindex_in_place` / `build_index` callbacks land.
    pub phase: Option<String>,
    pub current_file: Option<String>,
    pub files_done: u64,
    pub files_total: u64,
    pub error: Option<String>,
    /// Last N log lines emitted by the subprocess (S-005 follow-up
    /// monitor populates; Phase 1 stays empty unless test prefills).
    pub log_tail: Vec<String>,
    pub duration_ms: u64,
}

impl JobState {
    pub fn new_running() -> Self {
        Self {
            status: JobStatus::Running,
            percent: 0.0,
            phase: None,
            current_file: None,
            files_done: 0,
            files_total: 0,
            error: None,
            log_tail: Vec::new(),
            duration_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobHandle {
    pub job_id: String,
    pub slug: String,
    pub started_at: Instant,
    /// PID of the spawned subprocess (Phase 1 = Some when the launcher
    /// successfully spawned; None if the launcher couldn't or hasn't).
    /// Used by DELETE /reindex/:job_id → SIGTERM (AS-046).
    pub pid: Option<u32>,
    pub state: Arc<Mutex<JobState>>,
}

/// Atomic per-slug job registry. `try_insert` is the only entry point
/// that mutates — it returns either the new handle or the existing one
/// (race-safe per AS-021).
pub struct JobRegistry {
    inner: Mutex<HashMap<String, JobHandle>>,
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Result of `try_insert`. `Inserted` = caller owns the new job and
    /// should spawn the subprocess. `Existing` = another caller won the
    /// race; return 409 + same job_id so both clients can poll status.
    pub fn try_insert(&self, slug: &str) -> JobInsertResult {
        let mut guard = self.inner.lock().expect("JobRegistry mutex poisoned");
        if let Some(existing) = guard.get(slug) {
            return JobInsertResult::Existing(existing.clone());
        }
        let handle = JobHandle {
            job_id: gen_id(),
            slug: slug.to_string(),
            started_at: Instant::now(),
            pid: None,
            state: Arc::new(Mutex::new(JobState::new_running())),
        };
        guard.insert(slug.to_string(), handle.clone());
        JobInsertResult::Inserted(handle)
    }

    pub fn get(&self, slug: &str) -> Option<JobHandle> {
        self.inner
            .lock()
            .expect("JobRegistry mutex poisoned")
            .get(slug)
            .cloned()
    }

    pub fn remove(&self, slug: &str) -> Option<JobHandle> {
        self.inner
            .lock()
            .expect("JobRegistry mutex poisoned")
            .remove(slug)
    }

    /// AS-045 — find a handle by job_id (browser refresh resume). Linear
    /// scan; Phase 1 has at most ~10 active jobs so O(N) is fine. If
    /// hot-spot grows past ~100 jobs add a secondary index.
    pub fn lookup_by_id(&self, job_id: &str) -> Option<JobHandle> {
        let guard = self.inner.lock().expect("JobRegistry mutex poisoned");
        guard.values().find(|h| h.job_id == job_id).cloned()
    }

    /// Attach a PID to an existing handle (called by the spawn flow
    /// after `launcher.spawn_index` returns). Required for SIGTERM in
    /// AS-046.
    pub fn set_pid(&self, slug: &str, pid: u32) {
        let mut guard = self.inner.lock().expect("JobRegistry mutex poisoned");
        if let Some(h) = guard.get_mut(slug) {
            h.pid = Some(pid);
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobInsertResult {
    Inserted(JobHandle),
    Existing(JobHandle),
}

// ---------- Confirm tokens for 2-step DELETE (AS-022..AS-024) ----------

const CONFIRM_TTL: Duration = Duration::from_secs(30);

struct ConfirmEntry {
    token: String,
    issued_at: Instant,
}

pub struct ConfirmTokens {
    inner: Mutex<HashMap<String, ConfirmEntry>>,
}

impl Default for ConfirmTokens {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfirmTokens {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Issue a confirm token for `slug`. Existing token (if any) is
    /// replaced — calling delete-intent twice extends the window with a
    /// fresh token, prior token becomes invalid.
    pub fn issue(&self, slug: &str) -> (String, Duration) {
        let token = gen_id();
        let entry = ConfirmEntry {
            token: token.clone(),
            issued_at: Instant::now(),
        };
        self.inner
            .lock()
            .expect("ConfirmTokens mutex poisoned")
            .insert(slug.to_string(), entry);
        (token, CONFIRM_TTL)
    }

    /// Validate token. On success, **consume** the entry (one-shot use).
    pub fn validate(&self, slug: &str, token: &str) -> ConfirmResult {
        let mut guard = self.inner.lock().expect("ConfirmTokens mutex poisoned");
        let Some(entry) = guard.get(slug) else {
            return ConfirmResult::Missing;
        };
        if entry.issued_at.elapsed() > CONFIRM_TTL {
            guard.remove(slug);
            return ConfirmResult::Expired;
        }
        if entry.token != token {
            return ConfirmResult::Mismatch;
        }
        guard.remove(slug);
        ConfirmResult::Ok
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    Ok,
    Missing,
    Expired,
    Mismatch,
}

// ---------- JobLauncher abstraction ----------

/// Abstraction over "spawn `ga index` for this path". Real impl uses
/// `Command::new(absolute_ga_path).arg(canonical_path)`. Test impl just
/// records the call.
///
/// `state` is the JobState Arc the caller wants updated when the
/// subprocess transitions (exit success → `JobStatus::Done`, non-zero
/// → `JobStatus::Error`). The launcher owns the monitor task lifetime
/// — for `SubprocessLauncher` it's a `std::thread::spawn` that waits
/// on the child and writes the terminal status.
pub trait JobLauncher: Send + Sync + 'static {
    fn spawn_index(
        &self,
        repo_path: &Path,
        force: bool,
        state: Arc<Mutex<JobState>>,
    ) -> std::io::Result<u32>;
}

/// Default impl — argv list, no shell. Pin absolute path at construct
/// time (resolves via `current_exe()` so the binary stays consistent
/// even if PATH is hijacked).
pub struct SubprocessLauncher {
    pub graphatlas_bin: PathBuf,
}

impl SubprocessLauncher {
    /// Resolve `graphatlas` binary path from `current_exe()`. Caller is
    /// `ga-server` itself; the assumption is that the workspace ships
    /// `graphatlas` next to `ga-server` (cargo workspace install or
    /// dist tarball — Spec D D-C3 frontend resolution mirrors this).
    pub fn from_current_exe() -> std::io::Result<Self> {
        let me = std::env::current_exe()?;
        // Try sibling `graphatlas` first.
        let bin = me
            .parent()
            .map(|p| p.join("graphatlas"))
            .unwrap_or_else(|| PathBuf::from("graphatlas"));
        Ok(Self {
            graphatlas_bin: bin,
        })
    }
}

/// Drain NDJSON progress events from `reader` and stream them into
/// `state`. Each line is `{"phase":"<name>","percent":<f32>[, "files_done":N, "files_total":N]}`.
/// Malformed lines are kept in `log_tail` (so the user can see them in
/// the View Log panel) but otherwise ignored — we never poison the
/// parser thread on bad input. Stops at EOF; caller is responsible for
/// terminal status (the wait-monitor thread does that).
pub fn consume_progress<R: std::io::BufRead>(reader: R, state: Arc<Mutex<JobState>>) {
    // Cap chosen to keep View Log scannable + memory bounded.
    // `remove(0)` is O(n) on Vec but at cap=200 the memmove is negligible
    // vs the mutex acquire on the same line; VecDeque considered & rejected
    // — converting at the Vec<String> serialization boundary added more
    // complexity than the saved cycles.
    const LOG_TAIL_CAP: usize = 200;
    for line in reader.lines().flatten() {
        let parsed: Option<serde_json::Value> = serde_json::from_str(&line).ok();
        let mut st = state.lock().expect("JobState mutex poisoned");
        // Always keep the raw line in log_tail (capped).
        if st.log_tail.len() >= LOG_TAIL_CAP {
            st.log_tail.remove(0);
        }
        st.log_tail.push(line.clone());
        // Try to extract structured fields; non-JSON or non-progress
        // lines fall through with only the log entry recorded.
        if let Some(v) = parsed.as_ref().and_then(|v| v.as_object()) {
            if let Some(p) = v.get("percent").and_then(|p| p.as_f64()) {
                st.percent = p as f32;
            }
            if let Some(phase) = v.get("phase").and_then(|p| p.as_str()) {
                st.phase = Some(phase.to_string());
            }
            if let Some(d) = v.get("files_done").and_then(|n| n.as_u64()) {
                st.files_done = d;
            }
            if let Some(t) = v.get("files_total").and_then(|n| n.as_u64()) {
                st.files_total = t;
            }
        }
    }
}

impl JobLauncher for SubprocessLauncher {
    fn spawn_index(
        &self,
        repo_path: &Path,
        force: bool,
        state: Arc<Mutex<JobState>>,
    ) -> std::io::Result<u32> {
        // A-C6 invariant: argv list, no shell exec, env_clear + allowlist.
        let mut cmd = std::process::Command::new(&self.graphatlas_bin);
        cmd.arg("reindex").arg(repo_path).arg("--json-progress");
        let _ = force; // S-005 follow-up: `graphatlas reindex` always
                       // rebuilds in-place; no --force flag yet. Wire
                       // through when reindex grows incremental mode.
        cmd.env_clear();
        for (k, v) in std::env::vars() {
            if matches!(
                k.as_str(),
                "HOME" | "PATH" | "USER" | "LOGNAME" | "LANG" | "LC_ALL" | "TMPDIR" | "GRAPHATLAS_CACHE_DIR"
            ) {
                cmd.env(k, v);
            }
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let started = std::time::Instant::now();

        // Progress thread — drains NDJSON phase events from the child's
        // stdout into JobState as they arrive, so GET /status sees real
        // percent + log_tail before the child exits.
        let progress_handle = child.stdout.take().map(|stdout| {
            let progress_state = state.clone();
            std::thread::spawn(move || {
                consume_progress(std::io::BufReader::new(stdout), progress_state);
            })
        });

        // Stderr thread — captures up to STDERR_TAIL_CAP lines so a
        // non-zero exit can attach concrete context to JobState.error
        // (panic message, "cache file has unsafe permissions", etc.).
        // Without this, errors only surface in the parent's stderr
        // which is lost when ga-server runs detached.
        let stderr_buf: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_handle = child.stderr.take().map(|stderr| {
            let buf = stderr_buf.clone();
            let log_state = state.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                const STDERR_TAIL_CAP: usize = 50;
                for line in std::io::BufReader::new(stderr).lines().flatten() {
                    {
                        let mut b = buf.lock().expect("stderr buf mutex poisoned");
                        if b.len() >= STDERR_TAIL_CAP {
                            b.remove(0);
                        }
                        b.push(line.clone());
                    }
                    // Also feed into log_tail so View Log shows them
                    // interleaved with progress NDJSON.
                    let mut st = log_state.lock().expect("JobState mutex poisoned");
                    if st.log_tail.len() >= 200 {
                        st.log_tail.remove(0);
                    }
                    st.log_tail.push(format!("[stderr] {line}"));
                }
            })
        });

        // Monitor thread — waits for the subprocess, writes terminal
        // status to JobState. Joins the stdout/stderr drainers first
        // so a late buffered line from the child can't overwrite the
        // terminal `percent: 100` after we set it (H-1/M-1 race fix).
        std::thread::spawn(move || {
            let wait_result = child.wait();
            if let Some(h) = progress_handle {
                let _ = h.join();
            }
            if let Some(h) = stderr_handle {
                let _ = h.join();
            }
            let mut st = state.lock().expect("JobState mutex poisoned");
            st.duration_ms = started.elapsed().as_millis() as u64;
            match wait_result {
                Ok(status) => {
                    if status.success() {
                        st.status = JobStatus::Done;
                        st.percent = 100.0;
                    } else {
                        st.status = JobStatus::Error;
                        let tail = stderr_buf
                            .lock()
                            .expect("stderr buf mutex poisoned")
                            .join("\n");
                        let summary = if tail.is_empty() {
                            format!("reindex exited non-zero: {:?}", status.code())
                        } else {
                            format!(
                                "reindex exited non-zero (code {:?}):\n{tail}",
                                status.code()
                            )
                        };
                        st.error = Some(summary);
                    }
                }
                Err(e) => {
                    st.status = JobStatus::Error;
                    st.error = Some(format!("wait child: {e}"));
                }
            }
        });

        Ok(pid)
    }
}

// ---------- ID generation ----------

/// 32-hex-char job/confirm id. Plenty of entropy without pulling in
/// `uuid` just for this — we read from `/dev/urandom`-style source via
/// the OS getrandom path used by `Instant`+pid hashing isn't enough;
/// fall back to system time mixed with a counter for determinism in
/// tests. Real entropy comes from `std::collections::hash_map::RandomState`.
fn gen_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );
    let n1 = h.finish();
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(n1);
    let n2 = h2.finish();
    let mut buf = [0u8; 16];
    buf[..8].copy_from_slice(&n1.to_le_bytes());
    buf[8..].copy_from_slice(&n2.to_le_bytes());
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: SubprocessLauncher used to only wait for exit, so
    // JobState.percent sat at 0 throughout the entire reindex then
    // jumped to 100 — UI showed no progress (jobs.rs:296-319 comment
    // "Per-file progress is a Spec A follow-up").
    #[test]
    fn consume_progress_ndjson_advances_state_per_line() {
        use std::io::Cursor;
        let state = Arc::new(Mutex::new(JobState::new_running()));
        let ndjson = b"{\"phase\":\"indexing\",\"percent\":20.0}\n\
                       {\"phase\":\"graph\",\"percent\":60.0}\n\
                       {\"phase\":\"committing\",\"percent\":90.0,\"files_done\":42,\"files_total\":42}\n";
        consume_progress(Cursor::new(&ndjson[..]), state.clone());
        let st = state.lock().unwrap();
        assert_eq!(st.percent, 90.0);
        // L-1: phase identifier lands in `phase`, not `current_file`
        // (which is reserved for per-file progress).
        assert_eq!(st.phase.as_deref(), Some("committing"));
        assert_eq!(st.current_file, None);
        assert_eq!(st.files_done, 42);
        assert_eq!(st.files_total, 42);
        assert_eq!(st.log_tail.len(), 3);
        assert_eq!(st.status, JobStatus::Running, "consumer must not touch terminal status");
    }

    #[test]
    fn consume_progress_ignores_garbage_lines() {
        use std::io::Cursor;
        let state = Arc::new(Mutex::new(JobState::new_running()));
        let ndjson = b"Reindexed /x: 5 files in 12ms\n\
                       {\"phase\":\"indexing\",\"percent\":20.0}\n\
                       not-json-at-all\n";
        consume_progress(Cursor::new(&ndjson[..]), state.clone());
        let st = state.lock().unwrap();
        assert_eq!(st.percent, 20.0);
    }

    #[test]
    fn try_insert_returns_inserted_first_then_existing() {
        let reg = JobRegistry::new();
        let r1 = reg.try_insert("slug-a");
        assert!(matches!(r1, JobInsertResult::Inserted(_)));
        let r2 = reg.try_insert("slug-a");
        let (id_first, id_second) = match (r1, r2) {
            (JobInsertResult::Inserted(a), JobInsertResult::Existing(b)) => (a.job_id, b.job_id),
            _ => panic!("expected Inserted then Existing"),
        };
        assert_eq!(id_first, id_second, "Existing must echo the original job_id");
    }

    #[test]
    fn try_insert_concurrent_threads_only_one_inserts() {
        use std::sync::Arc;
        use std::thread;
        let reg = Arc::new(JobRegistry::new());
        let mut handles = Vec::new();
        for _ in 0..20 {
            let r = Arc::clone(&reg);
            handles.push(thread::spawn(move || r.try_insert("race-slug")));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let inserted: Vec<_> = results
            .iter()
            .filter(|r| matches!(r, JobInsertResult::Inserted(_)))
            .collect();
        assert_eq!(inserted.len(), 1, "exactly one thread should insert");
    }

    #[test]
    fn confirm_token_ok_then_consumed() {
        let store = ConfirmTokens::new();
        let (token, _ttl) = store.issue("slug-x");
        assert_eq!(store.validate("slug-x", &token), ConfirmResult::Ok);
        // Second use must fail — one-shot.
        assert_eq!(store.validate("slug-x", &token), ConfirmResult::Missing);
    }

    #[test]
    fn confirm_token_mismatch() {
        let store = ConfirmTokens::new();
        let (_token, _) = store.issue("slug-y");
        assert_eq!(store.validate("slug-y", "wrong"), ConfirmResult::Mismatch);
    }

    #[test]
    fn confirm_token_missing() {
        let store = ConfirmTokens::new();
        assert_eq!(store.validate("nope", "anything"), ConfirmResult::Missing);
    }

    #[test]
    fn gen_id_is_unique() {
        let a = gen_id();
        let b = gen_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
