//! Watcher control endpoint — Spec A S-006.
//!
//!   POST /api/projects/:slug/watcher   {action: "start"|"stop"}    AS-050 / AS-055
//!   GET  /api/projects/:slug/watcher                                status snapshot
//!
//! Driver-level side effects (notify-rs spawn) are routed through
//! `WatcherDriver` so tests don't touch the filesystem. State machine
//! lives here.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::recovery::find_cache_dir;
use crate::state::AppState;
use crate::watcher::{
    should_fallback_to_polling, StartOutcome, StopOutcome, WatcherMode, WatcherStatus,
};

#[derive(serde::Serialize)]
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

#[derive(Debug, Deserialize)]
pub struct WatcherActionBody {
    pub action: WatcherAction,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WatcherAction {
    Start,
    Stop,
}

pub async fn watcher_action(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<WatcherActionBody>,
) -> Response {
    // Project must exist.
    let cache_dir = match find_cache_dir(&state.cfg.cache_root, &slug) {
        Some(d) => d,
        None => return (StatusCode::NOT_FOUND, err("project_not_found", "")).into_response(),
    };
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

    let entry = state.watchers.entry(&slug);
    match body.action {
        WatcherAction::Start => {
            let outcome = state.watcher_driver.start(&slug, &repo_root);
            let mut guard = entry.lock().expect("WatcherEntry mutex poisoned");
            match outcome {
                StartOutcome::Started(mode) => {
                    guard.status = WatcherStatus::Running;
                    guard.mode = mode;
                    guard.error = None;
                }
                StartOutcome::FallbackPoll(reason) => {
                    guard.status = WatcherStatus::Running;
                    guard.mode = WatcherMode::Poll;
                    guard.error = Some(format!("Falling back to polling — {}", reason));
                    drop(guard);
                    let entry = state.watchers.entry(&slug);
                    let guard2 = entry.lock().unwrap();
                    return (StatusCode::OK, Json(guard2.snapshot())).into_response();
                }
                StartOutcome::Failed(msg) => {
                    // Retry with poll if the error pattern matches
                    // AS-053. Otherwise mark Errored.
                    if should_fallback_to_polling(&msg) {
                        guard.status = WatcherStatus::Running;
                        guard.mode = WatcherMode::Poll;
                        guard.error = Some(format!("Falling back to polling — {}", msg));
                    } else {
                        guard.status = WatcherStatus::Errored;
                        guard.error = Some(msg);
                    }
                }
            }
            let snap = guard.snapshot();
            (StatusCode::OK, Json(snap)).into_response()
        }
        WatcherAction::Stop => {
            let outcome = state.watcher_driver.stop(&slug);
            let mut guard = entry.lock().expect("WatcherEntry mutex poisoned");
            guard.status = WatcherStatus::Stopped;
            guard.queue_pending = 0;
            guard.dirty_flag = false;
            guard.error = match outcome {
                StopOutcome::KilledAfterTimeout => Some("watcher killed after join timeout".into()),
                _ => None,
            };
            let snap = guard.snapshot();
            (StatusCode::OK, Json(snap)).into_response()
        }
    }
}

pub async fn watcher_status(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    // Project must exist.
    if find_cache_dir(&state.cfg.cache_root, &slug).is_none() {
        return (StatusCode::NOT_FOUND, err("project_not_found", "")).into_response();
    }
    let entry = state.watchers.entry(&slug);
    let snap = entry
        .lock()
        .expect("WatcherEntry mutex poisoned")
        .snapshot();
    (StatusCode::OK, Json(snap)).into_response()
}

fn read_repo_root(cache_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let bytes = std::fs::read(cache_dir.join("metadata.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(std::path::PathBuf::from(v.get("repo_root")?.as_str()?))
}
