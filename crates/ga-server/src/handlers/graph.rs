//! Graph + symbol detail endpoints — Spec A S-004.
//!
//! All handlers go through `ProjectDataSource` so tests can swap in a
//! FakeDataSource without touching lbug. The route layer also enforces:
//!   - AS-040 `X-GA-Stale: reindex-in-progress` when JobRegistry has an
//!     active job for the slug.
//!   - AS-041 503 when cache state is Corrupt.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::cache_state::{lookup_cache_state, CacheState};
use crate::data::{
    clamp_limit, clamp_search_limit, is_safe_layer_name, is_safe_pattern, DataError,
};
use crate::state::AppState;

// ============== Query strings ==============

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    pub focus: Option<String>,
    #[serde(default = "default_hops")]
    pub hops: u8,
}
fn default_hops() -> u8 {
    2
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct SymbolsQuery {
    pub q: Option<String>,
    pub limit: Option<u64>,
}

// ============== Common helpers ==============

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

/// Pre-gate every data endpoint: cache-state probe + X-GA-Stale header
/// (AS-040 / AS-041). Returns either:
///   * `Err(response)` — handler should return this immediately.
///   * `Ok(headers)`   — augmenting headers to attach on the success path.
#[allow(clippy::result_large_err)] // axum Response is intentionally large; boxing would force unwrap at every callsite
fn pre_gate(state: &AppState, slug: &str) -> Result<HeaderMap, Response> {
    // AS-041 cache state.
    match lookup_cache_state(&state.cfg.cache_root, slug) {
        CacheState::NotFound => {
            return Err((StatusCode::NOT_FOUND, err("project_not_found", "")).into_response());
        }
        CacheState::Corrupt => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                err("cache_corrupt", "reindex required"),
            )
                .into_response());
        }
        CacheState::Building => {
            // Spec C C-cross-3 hint — block reads while still building
            // because there's no committed data yet. AS-041 wording
            // covers this too (UI buộc reindex).
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                err("cache_building", "cache is being built; retry shortly"),
            )
                .into_response());
        }
        CacheState::Fresh => {}
    }

    // AS-040 X-GA-Stale header injection.
    let mut headers = HeaderMap::new();
    if state.jobs.get(slug).is_some() {
        headers.insert(
            "x-ga-stale",
            HeaderValue::from_static("reindex-in-progress"),
        );
    }
    Ok(headers)
}

fn map_data_err(e: DataError) -> Response {
    let status = e.status_code();
    (status, err(e.error_code(), e.message())).into_response()
}

fn ok_with_headers<T: serde::Serialize>(headers: HeaderMap, body: T) -> Response {
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    for (k, v) in headers.iter() {
        resp.headers_mut().insert(k, v.clone());
    }
    resp.headers_mut()
        .entry(header::CONTENT_TYPE)
        .or_insert(HeaderValue::from_static("application/json"));
    resp
}

// ============== Handlers ==============

// AS-032 / AS-033 / AS-034
pub async fn graph_endpoint(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<GraphQuery>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let hops = q.hops.clamp(1, 2);
    match state.data.graph_dump(&slug, q.focus.as_deref(), hops) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// AS-035 / AS-038
pub async fn symbol_detail_endpoint(
    State(state): State<AppState>,
    Path((slug, symbol_id)): Path<(String, String)>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match state.data.symbol_detail(&slug, &symbol_id) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// AS-036 / AS-037
pub async fn callers_endpoint(
    State(state): State<AppState>,
    Path((slug, symbol_id)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let offset = q.offset.unwrap_or(0);
    let limit = clamp_limit(q.limit);
    match state.data.callers(&slug, &symbol_id, offset, limit) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

pub async fn callees_endpoint(
    State(state): State<AppState>,
    Path((slug, symbol_id)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let offset = q.offset.unwrap_or(0);
    let limit = clamp_limit(q.limit);
    match state.data.callees(&slug, &symbol_id, offset, limit) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

pub async fn importers_endpoint(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(file): Query<FileQuery>,
    Query(page): Query<PageQuery>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let offset = page.offset.unwrap_or(0);
    let limit = clamp_limit(page.limit);
    match state.data.importers(&slug, &file.path, offset, limit) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// ====================== Spec E ======================

// Spec E AS-001 / AS-002 / AS-003 — global symbol search.
pub async fn symbols_search_endpoint(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<SymbolsQuery>,
) -> Response {
    let pattern = q.q.unwrap_or_default();
    if !is_safe_pattern(&pattern) {
        return (
            StatusCode::BAD_REQUEST,
            err(
                "bad_pattern",
                "pattern contains characters outside [A-Za-z0-9_$.]",
            ),
        )
            .into_response();
    }
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let limit = clamp_search_limit(q.limit);
    match state.data.symbols_search(&slug, &pattern, limit) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// Spec E AS-006 / AS-007 — module list (layer chip strip).
pub async fn layers_endpoint(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match state.data.layers(&slug) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// Spec E AS-008 / AS-009 — per-layer symbol list (lazy expand).
pub async fn layer_symbols_endpoint(
    State(state): State<AppState>,
    Path((slug, layer_name)): Path<(String, String)>,
) -> Response {
    if !is_safe_layer_name(&layer_name) {
        return (
            StatusCode::BAD_REQUEST,
            err("bad_pattern", "layer name contains unsafe characters"),
        )
            .into_response();
    }
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match state.data.layer_symbols(&slug, &layer_name) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}

// AS-039
pub async fn file_summary_endpoint(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(file): Query<FileQuery>,
) -> Response {
    let headers = match pre_gate(&state, &slug) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match state.data.file_summary(&slug, &file.path) {
        Ok(body) => ok_with_headers(headers, body),
        Err(e) => map_data_err(e),
    }
}
