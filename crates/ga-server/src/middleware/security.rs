//! Security middleware — Origin / Host gates.
//!
//! AS coverage:
//!   AS-003 — Origin allowlist (403 bad_origin)
//!   AS-004 — Host validation  (421 misdirected_request)
//!
//! Token check (AS-005) was removed 2026-05-17: server binds loopback
//! only; Origin + Host validation already block CSRF + DNS rebinding,
//! so the token added UX friction without a corresponding threat. See
//! Spec D Change Log entry for the security model trade-off note.

use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::AppState;

fn err_response(status: StatusCode, code: &'static str, msg: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": code, "message": msg })),
    )
        .into_response()
}

fn header_str<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key).and_then(|v| v.to_str().ok())
}

/// Axum `from_fn_with_state` handler. Phase 1 contract:
/// • Requests must carry Origin in `cfg.allowed_origins` (state-changing methods).
/// • Requests must carry Host in `cfg.allowed_hosts`.
///
/// Health endpoint is routed OUTSIDE this layer (see lib.rs build_app),
/// so it never sees this function.
pub async fn enforce<B>(
    State(state): State<AppState>,
    req: Request<B>,
    next: Next,
) -> Response
where
    B: Send + 'static,
    Request<B>: Into<Request<axum::body::Body>>,
{
    let headers = req.headers();
    let method = req.method();

    // AS-003 — Origin allowlist.
    //
    // Browsers don't send `Origin` on same-origin GET / HEAD (Fetch spec
    // §3.1). For state-changing methods we still require Origin — the
    // CSRF risk vector is POST/DELETE from a malicious tab.
    let require_origin = matches!(
        method.as_str(),
        "POST" | "PUT" | "DELETE" | "PATCH"
    );
    if let Some(origin) = header_str(headers, "origin") {
        if !state.cfg.allowed_origins.iter().any(|a| a == origin) {
            return err_response(
                StatusCode::FORBIDDEN,
                "bad_origin",
                "Origin not in allowlist",
            );
        }
    } else if require_origin {
        return err_response(
            StatusCode::FORBIDDEN,
            "bad_origin",
            "Origin header required on non-GET (AS-003)",
        );
    }

    // AS-004 — Host validation (DNS rebinding defense).
    let host = match header_str(headers, "host") {
        Some(h) => h,
        None => {
            return err_response(
                StatusCode::MISDIRECTED_REQUEST,
                "bad_host",
                "Host header required (AS-004)",
            );
        }
    };
    if !state.cfg.allowed_hosts.iter().any(|allowed| allowed == host) {
        return err_response(
            StatusCode::MISDIRECTED_REQUEST,
            "bad_host",
            "Host not in allowlist",
        );
    }

    let req: Request<axum::body::Body> = req.into();
    next.run(req).await
}
