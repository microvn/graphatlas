//! ga-server — HTTP backend cho `ga ui` (Spec A).
//!
//! Phase 1 chỉ bind `127.0.0.1`. Mọi request non-GET hoặc non-`/api/health`
//! phải qua security middleware:
//!   1. Origin header allowlist (chống CSRF từ browser tab cross-origin)
//!   2. Host header validation (chống DNS rebinding)
//!   3. `X-GA-Token` per-session token (custom header force preflight)
//!
//! S-001 ship: bootstrap + middleware + `/api/health` + `/api/config`.
//! Story S-002..S-006 sẽ thêm router branches nhưng middleware giữ
//! nguyên — security là invariant.

pub mod cache_state;
pub mod config;
pub mod data;
pub mod handlers;
pub mod jobs;
pub mod lbug_source;
pub mod middleware;
pub mod paths;
pub mod recovery;
pub mod state;
pub mod watcher;

use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Serialize;

pub use config::ServerConfig;
pub use data::ProjectDataSource;
pub use jobs::{JobLauncher, JobRegistry, SubprocessLauncher};
pub use lbug_source::LbugDataSource;
pub use state::AppState;

/// Build axum router with security middleware applied to all non-health
/// routes. Health endpoint stays outside middleware so monitoring +
/// `ga ui` health probe (Spec D AS-001) work without a token.
pub fn build_app(state: AppState) -> Router {
    use handlers::{graph, projects, reindex, watcher as watcher_h};
    // Protected routes: must pass security middleware.
    let protected = Router::new()
        .route("/api/config", get(handler_config))
        .route(
            "/api/projects",
            get(projects::list_projects).post(projects::add_project),
        )
        .route(
            "/api/projects/:slug/delete-intent",
            post(projects::delete_intent),
        )
        .route("/api/projects/:slug", delete(projects::delete_project))
        // S-004 data endpoints.
        .route("/api/projects/:slug/graph", get(graph::graph_endpoint))
        .route(
            "/api/projects/:slug/symbol/:symbol_id",
            get(graph::symbol_detail_endpoint),
        )
        .route(
            "/api/projects/:slug/symbol/:symbol_id/callers",
            get(graph::callers_endpoint),
        )
        .route(
            "/api/projects/:slug/symbol/:symbol_id/callees",
            get(graph::callees_endpoint),
        )
        .route(
            "/api/projects/:slug/importers",
            get(graph::importers_endpoint),
        )
        .route(
            "/api/projects/:slug/file",
            get(graph::file_summary_endpoint),
        )
        // Spec E search + layers.
        .route(
            "/api/projects/:slug/symbols",
            get(graph::symbols_search_endpoint),
        )
        .route("/api/projects/:slug/layers", get(graph::layers_endpoint))
        .route(
            "/api/projects/:slug/layers/:layer_name/symbols",
            get(graph::layer_symbols_endpoint),
        )
        // S-005 reindex job lifecycle.
        .route("/api/projects/:slug/reindex", post(reindex::start_reindex))
        .route(
            "/api/projects/:slug/reindex/:job_id/status",
            get(reindex::job_status),
        )
        .route(
            "/api/projects/:slug/reindex/:job_id",
            delete(reindex::cancel_reindex),
        )
        // S-006 watcher control.
        .route(
            "/api/projects/:slug/watcher",
            get(watcher_h::watcher_status).post(watcher_h::watcher_action),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::security::enforce,
        ));

    Router::new()
        // Unprotected: only the loopback liveness probe.
        .route("/api/health", get(handler_health))
        .merge(protected)
        .with_state(state)
}

/// Build app with an optional static file root mounted at `/`. Phase 1
/// `ga ui` stand-in until Bun frontend ships (D-S001 follow-up close).
pub fn build_app_with_static(state: AppState, ui_dir: Option<std::path::PathBuf>) -> Router {
    let app = build_app(state);
    match ui_dir {
        Some(dir) if dir.is_dir() => {
            // Serve static at `/`. The protected /api routes already
            // matched above, so they take precedence — the static
            // service only catches what isn't an API route.
            let static_service = tower_http::services::ServeDir::new(&dir);
            app.fallback_service(static_service)
        }
        _ => app,
    }
}

/// AS-002 — refuse non-loopback bind at the API level. The CLI layer
/// also rejects, but defense-in-depth: a library caller embedding
/// `serve()` cannot accidentally bind 0.0.0.0.
pub fn validate_bind_addr(addr: &SocketAddr) -> Result<(), String> {
    if !addr.ip().is_loopback() {
        return Err(format!(
            "Phase 1 bind 127.0.0.1 only; got {} (per Spec A AS-002 / C-cross-2)",
            addr.ip()
        ));
    }
    Ok(())
}

/// Run the server. Spawns the axum binding on the provided socket.
/// Returns once the listener is bound; the caller drives the future to
/// completion (typically `tokio::signal::ctrl_c`).
pub async fn serve(state: AppState, addr: SocketAddr) -> anyhow::Result<()> {
    validate_bind_addr(&addr).map_err(anyhow::Error::msg)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {} failed: {} (AS-006 port conflict?)", addr, e))?;
    tracing::info!(target: "ga_server", "listening on http://{}", addr);
    axum::serve(listener, build_app(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!(target: "ga_server", "shutdown signal received");
        })
        .await
        .map_err(Into::into)
}

// ---------- Handlers ----------

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

async fn handler_health() -> impl IntoResponse {
    let body = HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    };
    (StatusCode::OK, Json(body))
}

#[derive(Serialize)]
struct ConfigResponse {
    server_version: &'static str,
    frontend_origin_expected: String,
}

async fn handler_config(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    let body = ConfigResponse {
        server_version: env!("CARGO_PKG_VERSION"),
        frontend_origin_expected: state.cfg.frontend_origin.clone(),
    };
    (StatusCode::OK, Json(body))
}
