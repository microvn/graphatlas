//! Reindex polling endpoints — Spec A S-005.
//!
//!   POST   /api/projects/:slug/reindex                      AS-042 / AS-043 / AS-044 / AS-048
//!   GET    /api/projects/:slug/reindex/:job_id/status       AS-045
//!   DELETE /api/projects/:slug/reindex/:job_id              AS-046
//!
//! State transitions live in `jobs::JobState`. The subprocess monitor
//! task (Spec A S-005 follow-up) drives the state machine. Phase 1
//! ships the route surface + state machine + tests via direct registry
//! manipulation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::cache_state::{lookup_cache_state, CacheState};
use crate::jobs::{JobInsertResult, JobStatus};
use crate::paths::PathRejection;
use crate::recovery::{self, ReindexPidFile};
use crate::state::AppState;

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}
fn err(code: &'static str, message: impl Into<String>) -> Json<ErrorBody> {
    Json(ErrorBody {
        error: code,
        message: message.into(),
    })
}

#[derive(Serialize)]
struct ReindexStartResponse {
    slug: String,
    job_id: String,
}

#[derive(Serialize)]
struct ReindexStatusResponse {
    job_id: String,
    slug: String,
    state: JobStatus,
    percent: f32,
    /// Indexer phase identifier (`opening`, `indexing`, `graph`,
    /// `committing`, `done`). Coarse milestone progress.
    phase: Option<String>,
    /// Per-file progress (filename). Reserved for when deep
    /// `reindex_in_place` / `build_index` callbacks land; phase 1
    /// leaves this null and uses `phase` instead.
    current_file: Option<String>,
    files_done: u64,
    files_total: u64,
    duration_ms: u64,
    error: Option<String>,
    log_tail: Vec<String>,
}

// ============== POST /api/projects/:slug/reindex (AS-042..AS-044, AS-048) ==============

pub async fn start_reindex(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    // Project must exist on disk + not already corrupt-blocked.
    let cache_dir = match recovery::find_cache_dir(&state.cfg.cache_root, &slug) {
        Some(d) => d,
        None => {
            return (StatusCode::NOT_FOUND, err("project_not_found", "")).into_response();
        }
    };
    // We allow starting a reindex *out of* Corrupt state (that's the
    // intended recovery path). Building means one of:
    //   (a) a live job is in our JobRegistry → `try_insert` below
    //       returns 409 with the registry's job_id.
    //   (b) JobRegistry empty BUT pidfile points to a still-alive
    //       grandchild (post-server-restart orphan, AS-049 case) →
    //       return 409 with the pidfile's job_id; do NOT unlink it
    //       or we'd spawn a second writer racing the live one on flock.
    //   (c) JobRegistry empty AND pidfile is dead/missing → genuine
    //       stale flag; unlink any leftover pidfile and proceed.
    // We deliberately do NOT call recovery::apply_cleanup here: writing
    // `index_state: "corrupt"` would make ga_core's strict parser
    // refuse to open the cache, preventing the reindex from starting.
    if matches!(
        lookup_cache_state(&state.cfg.cache_root, &slug),
        CacheState::Building
    ) && state.jobs.get(&slug).is_none()
    {
        if let Some(pf) = recovery::read_pid_file(&cache_dir) {
            if recovery::pid_alive(pf.pid) {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "reindex_in_progress",
                        "slug": pf.slug,
                        "job_id": pf.job_id,
                    })),
                )
                    .into_response();
            }
        }
        // Pidfile dead or absent → safe to clear.
        let _ = std::fs::remove_file(cache_dir.join(".reindex.pid"));
    }

    // Resolve repo_root from metadata so we can hand the canonical path
    // to the launcher (argv list, no shell — A-C6 invariant).
    let repo_root = match read_repo_root(&cache_dir) {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                err("metadata_unreadable", ""),
            )
                .into_response();
        }
    };
    // Defense in depth — same path-safety rules as POST /api/projects.
    // Canonical form is what we hand to the launcher (AS-048 invariant).
    let canonical_repo_root =
        match crate::paths::validate_repo_path(&repo_root, &state.cfg.cache_root) {
            Ok(p) => p,
            Err(rej) => {
                let msg = match &rej {
                    PathRejection::NotFound => "repo path missing on disk",
                    PathRejection::NotDirectory => "repo path is not a directory",
                    PathRejection::Unsafe(_) => "repo path failed safety check",
                    PathRejection::ExternalSymlink(_) => "repo path contains external symlink",
                };
                return (StatusCode::BAD_REQUEST, err(rej.code(), msg)).into_response();
            }
        };

    match state.jobs.try_insert(&slug) {
        JobInsertResult::Existing(handle) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "reindex_in_progress",
                "slug": handle.slug,
                "job_id": handle.job_id,
            })),
        )
            .into_response(),
        JobInsertResult::Inserted(handle) => {
            // Spawn subprocess. Real impl monitor loop = S-005 follow-up;
            // here we just record PID + write .reindex.pid so AS-049
            // recovery is correct.
            match state
                .launcher
                .spawn_index(&canonical_repo_root, true, handle.state.clone())
            {
                Ok(pid) => {
                    state.jobs.set_pid(&slug, pid);
                    let pf = ReindexPidFile {
                        pid,
                        job_id: handle.job_id.clone(),
                        slug: handle.slug.clone(),
                        started_at_unix: now_unix(),
                    };
                    let _ = recovery::write_pid_file(&cache_dir, &pf);
                    (
                        StatusCode::ACCEPTED,
                        Json(ReindexStartResponse {
                            slug: handle.slug,
                            job_id: handle.job_id,
                        }),
                    )
                        .into_response()
                }
                Err(e) => {
                    state.jobs.remove(&slug);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        err("spawn_failed", e.to_string()),
                    )
                        .into_response()
                }
            }
        }
    }
}

// ============== GET /api/projects/:slug/reindex/:job_id/status (AS-045) ==============

pub async fn job_status(
    State(state): State<AppState>,
    Path((_slug, job_id)): Path<(String, String)>,
) -> Response {
    let Some(handle) = state.jobs.lookup_by_id(&job_id) else {
        return (StatusCode::NOT_FOUND, err("job_not_found", "")).into_response();
    };
    let snap = handle
        .state
        .lock()
        .expect("JobState mutex poisoned")
        .clone();
    let body = ReindexStatusResponse {
        job_id: handle.job_id,
        slug: handle.slug,
        state: snap.status,
        percent: snap.percent,
        phase: snap.phase,
        current_file: snap.current_file,
        files_done: snap.files_done,
        files_total: snap.files_total,
        duration_ms: snap.duration_ms,
        error: snap.error,
        log_tail: snap.log_tail,
    };
    (StatusCode::OK, Json(body)).into_response()
}

// ============== DELETE /api/projects/:slug/reindex/:job_id (AS-046) ==============

pub async fn cancel_reindex(
    State(state): State<AppState>,
    Path((slug, job_id)): Path<(String, String)>,
) -> Response {
    let Some(handle) = state.jobs.lookup_by_id(&job_id) else {
        return (StatusCode::NOT_FOUND, err("job_not_found", "")).into_response();
    };
    if handle.slug != slug {
        return (
            StatusCode::NOT_FOUND,
            err("job_slug_mismatch", "job_id belongs to a different slug"),
        )
            .into_response();
    }

    // SIGTERM the subprocess if we have a PID. Workspace forbids unsafe;
    // shell out to `kill` via Command (same pattern as cmd_ui pid probe).
    if let Some(pid) = handle.pid {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }

    // Flip state → Cancelled.
    {
        let mut st = handle.state.lock().expect("JobState mutex poisoned");
        st.status = JobStatus::Cancelled;
        st.duration_ms = handle.started_at.elapsed().as_millis() as u64;
    }

    // Mark cache Corrupt per Spec A AS-046 / A-C8 / C-cross-8.
    if let Some(cache_dir) = recovery::find_cache_dir(&state.cfg.cache_root, &slug) {
        let _ = recovery::apply_cleanup(&cache_dir);
    }

    // Drop the slug → handle binding so the next POST /reindex starts
    // fresh. We DO NOT delete from lookup_by_id history immediately —
    // the client may still poll once more to see Cancelled.
    state.jobs.remove(&slug);

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "job_id": job_id,
            "state": "Cancelled",
        })),
    )
        .into_response()
}

// ============== helpers ==============

fn read_repo_root(cache_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let bytes = std::fs::read(cache_dir.join("metadata.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let s = v.get("repo_root")?.as_str()?;
    Some(std::path::PathBuf::from(s))
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
