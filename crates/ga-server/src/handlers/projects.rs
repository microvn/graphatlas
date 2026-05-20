//! Projects registry handlers — Spec A S-002.
//!
//! Endpoints:
//!   GET    /api/projects                          (AS-010..012, AS-025)
//!   POST   /api/projects                          (AS-013..021)
//!   POST   /api/projects/:slug/delete-intent      (AS-022)
//!   DELETE /api/projects/:slug                    (AS-022..024)
//!
//! Response DTOs live in `projects_types.rs` to keep handler logic
//! focused. ProjectRow Phase 1 omits `index_counts` and `health`
//! (None until S-003 migration writes them).

use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::handlers::projects_types::{
    ErrorBody, ProjectIndexState, ProjectRow, WatcherStatus,
};
use crate::paths::{validate_repo_path, PathRejection};
use crate::state::AppState;

// ---------- GET /api/projects ----------

pub async fn list_projects(State(state): State<AppState>) -> impl IntoResponse {
    let (rows, corrupt_count) = scan_projects(&state.cfg.cache_root);
    let mut headers = HeaderMap::new();
    if corrupt_count > 0 {
        headers.insert(
            "x-ga-corrupt-count",
            HeaderValue::from_str(&corrupt_count.to_string()).unwrap(),
        );
    }
    (StatusCode::OK, headers, Json(rows))
}

/// Walk `cache_root` via `ga_index::list::list_caches`, then layer
/// runtime derivations: orphan detection, watcher placeholder.
/// Returns `(rows, corrupt_count)` so AS-011 can emit
/// `X-GA-Corrupt-Count` by diffing on-disk dir entries against the
/// list_caches output (it skips bad metadata silently).
fn scan_projects(cache_root: &std::path::Path) -> (Vec<ProjectRow>, u64) {
    let listed = ga_index::list::list_caches(cache_root).unwrap_or_default();
    let listed_set: std::collections::HashSet<_> =
        listed.iter().map(|e| e.dir_name.clone()).collect();

    let mut corrupt = 0u64;
    if let Ok(entries) = std::fs::read_dir(cache_root) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let dname = match p.file_name().and_then(|n| n.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if listed_set.contains(&dname) {
                continue;
            }
            if p.join("metadata.json").is_file() {
                corrupt += 1;
            }
        }
    }

    let mut rows: Vec<ProjectRow> = listed
        .into_iter()
        .map(|e| {
            let path_missing = !std::path::Path::new(&e.repo_root).exists();
            let index_state = if path_missing {
                ProjectIndexState::Orphan
            } else {
                match e.index_state {
                    ga_core::IndexState::Complete => ProjectIndexState::Fresh,
                    ga_core::IndexState::Building => ProjectIndexState::Building,
                }
            };
            let name = std::path::Path::new(&e.repo_root)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&e.repo_root)
                .to_string();
            // Cache dir name `<name>-<6hex>` per Foundation-C12 — use
            // trailing piece as slug.
            let slug = e
                .dir_name
                .rsplit('-')
                .next()
                .map(|s| s.to_string())
                .unwrap_or_else(|| e.dir_name.clone());
            let languages: Vec<crate::handlers::projects_types::LangCount> = e
                .cache_lang_set
                .iter()
                .map(|l| crate::handlers::projects_types::LangCount {
                    lang: format!("{:?}", l).to_lowercase(),
                    // Per-language file count not currently tracked in
                    // metadata; UI hides zero counts. Wire to a real
                    // per-lang tally in a future S-003 enhancement.
                    file_count: 0,
                })
                .collect();
            let index_counts =
                e.index_counts
                    .as_ref()
                    .map(|c| crate::handlers::projects_types::IndexCounts {
                        node_count: c.node_count,
                        edge_count: c.edge_count,
                        file_count: c.file_count,
                        last_index_duration_ms: c.last_index_duration_ms,
                        db_size_bytes: c.db_size_bytes,
                    });
            let health = e.health_summary.as_ref().map(|h| {
                crate::handlers::projects_types::HealthSummary {
                    computed_at_unix: h.computed_at_unix,
                    hubs_count: h.hubs_count,
                    bridges_count: h.bridges_count,
                    dead_code_count: h.dead_code_count,
                    large_functions_count: h.large_functions_count,
                    tested_count: h.tested_count,
                }
            });
            ProjectRow {
                slug,
                name,
                repo_root: e.repo_root,
                languages,
                last_indexed_unix: e.last_indexed_unix,
                index_state,
                index_counts,
                health,
                watcher: WatcherStatus::Stopped,
                watcher_queue_pending: 0,
                watcher_last_event_unix: None,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.last_indexed_unix.cmp(&a.last_indexed_unix));
    (rows, corrupt)
}

// ---------- POST /api/projects ----------

#[derive(Debug, Deserialize)]
pub struct AddProjectRequest {
    pub path: String,
    #[serde(default = "default_mode")]
    pub mode: AddMode,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AddMode {
    Index,
    Attach,
}

fn default_mode() -> AddMode {
    AddMode::Index
}

#[derive(Debug, serde::Serialize)]
struct AddProjectResponse {
    slug: String,
    job_id: Option<String>,
    mode: String,
    canonical_path: String,
}

fn err_body(code: &str, msg: &str) -> Json<ErrorBody> {
    Json(ErrorBody {
        error: code.into(),
        message: msg.into(),
    })
}

pub async fn add_project(
    State(state): State<AppState>,
    Json(req): Json<AddProjectRequest>,
) -> impl IntoResponse {
    let input = PathBuf::from(&req.path);
    let canonical = match validate_repo_path(&input, &state.cfg.cache_root) {
        Ok(p) => p,
        Err(rej) => {
            let status = match rej {
                PathRejection::NotFound | PathRejection::NotDirectory => StatusCode::BAD_REQUEST,
                PathRejection::Unsafe(_) | PathRejection::ExternalSymlink(_) => {
                    StatusCode::BAD_REQUEST
                }
            };
            return (status, err_body(rej.code(), &rej.message())).into_response();
        }
    };

    let slug = slug_for(&canonical);
    let cache_exists = cache_dir_exists(&state.cfg.cache_root, &slug);

    if matches!(req.mode, AddMode::Attach) {
        if !cache_exists {
            return (
                StatusCode::BAD_REQUEST,
                err_body(
                    "cache_not_found",
                    "no existing cache for this path; use mode=index to build one",
                ),
            )
                .into_response();
        }
        return (
            StatusCode::OK,
            Json(AddProjectResponse {
                slug,
                job_id: None,
                mode: "attach".into(),
                canonical_path: canonical.display().to_string(),
            }),
        )
            .into_response();
    }

    // mode=index — race-safe spawn via JobRegistry try_insert.
    match state.jobs.try_insert(&slug) {
        crate::jobs::JobInsertResult::Inserted(handle) => {
            let force = cache_exists; // already-cached + index = reindex
            match state
                .launcher
                .spawn_index(&canonical, force, handle.state.clone())
            {
                Ok(_pid) => (
                    StatusCode::ACCEPTED,
                    Json(AddProjectResponse {
                        slug: handle.slug,
                        job_id: Some(handle.job_id),
                        mode: "index".into(),
                        canonical_path: canonical.display().to_string(),
                    }),
                )
                    .into_response(),
                Err(e) => {
                    // Release the slot so retries can succeed.
                    state.jobs.remove(&slug);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        err_body("spawn_failed", &e.to_string()),
                    )
                        .into_response()
                }
            }
        }
        crate::jobs::JobInsertResult::Existing(handle) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "reindex_in_progress",
                "job_id": handle.job_id,
                "slug": handle.slug,
            })),
        )
            .into_response(),
    }
}

// ---------- POST /api/projects/:slug/delete-intent ----------

#[derive(Debug, serde::Serialize)]
struct DeleteIntentResponse {
    confirm_token: String,
    expires_in_secs: u64,
}

pub async fn delete_intent(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    if !cache_dir_exists(&state.cfg.cache_root, &slug) {
        return (StatusCode::NOT_FOUND, err_body("project_not_found", "")).into_response();
    }
    let (token, ttl) = state.confirm_tokens.issue(&slug);
    (
        StatusCode::OK,
        Json(DeleteIntentResponse {
            confirm_token: token,
            expires_in_secs: ttl.as_secs(),
        }),
    )
        .into_response()
}

// ---------- DELETE /api/projects/:slug?confirm=<token> ----------

#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    pub confirm: Option<String>,
}

pub async fn delete_project(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> impl IntoResponse {
    let Some(token) = q.confirm else {
        return (
            StatusCode::FORBIDDEN,
            err_body(
                "missing_confirm_token",
                "DELETE requires ?confirm=<token>; call POST /:slug/delete-intent first",
            ),
        )
            .into_response();
    };
    match state.confirm_tokens.validate(&slug, &token) {
        crate::jobs::ConfirmResult::Ok => {}
        crate::jobs::ConfirmResult::Expired => {
            return (
                StatusCode::FORBIDDEN,
                err_body("confirm_token_expired", "token TTL elapsed; reissue intent"),
            )
                .into_response()
        }
        crate::jobs::ConfirmResult::Mismatch | crate::jobs::ConfirmResult::Missing => {
            return (
                StatusCode::FORBIDDEN,
                err_body("invalid_confirm_token", ""),
            )
                .into_response()
        }
    }

    // Defense-in-depth: assert ancestry before rm.
    let cache_root_canonical = state
        .cfg
        .cache_root
        .canonicalize()
        .unwrap_or_else(|_| state.cfg.cache_root.clone());
    let Some(cache_dir) = find_cache_dir(&state.cfg.cache_root, &slug) else {
        return (StatusCode::NOT_FOUND, err_body("project_not_found", "")).into_response();
    };
    let canonical = match cache_dir.canonicalize() {
        Ok(c) => c,
        Err(_) => return (StatusCode::NOT_FOUND, err_body("project_not_found", "")).into_response(),
    };
    if !canonical.starts_with(&cache_root_canonical) {
        return (
            StatusCode::FORBIDDEN,
            err_body("path_unsafe", "cache dir escaped cache_root"),
        )
            .into_response();
    }
    if let Err(e) = std::fs::remove_dir_all(&canonical) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            err_body("delete_failed", &e.to_string()),
        )
            .into_response();
    }
    state.jobs.remove(&slug);
    (StatusCode::NO_CONTENT, ()).into_response()
}

// ---------- helpers ----------

/// Slug derivation = first 8 hex chars of blake3(canonical path).
/// Matches Foundation-C12 cache naming shape (`<name>-<6hex>`); new
/// caches haven't been written yet at POST time so we synthesize.
pub fn slug_for(canonical: &std::path::Path) -> String {
    let hash = blake3::hash(canonical.to_string_lossy().as_bytes());
    hex::encode(&hash.as_bytes()[..4])
}

fn cache_dir_exists(cache_root: &std::path::Path, slug: &str) -> bool {
    find_cache_dir(cache_root, slug).is_some()
}

fn find_cache_dir(cache_root: &std::path::Path, slug: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(cache_root).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(slug) || name == slug {
            return Some(p);
        }
    }
    None
}
