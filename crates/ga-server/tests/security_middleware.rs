//! Integration tests for ga-server Spec A S-001 security middleware.
//!
//! Each test names the AS it exercises. Coverage map:
//!   AS-001 — health OK without token + protected route 403 without token
//!   AS-003 — Origin allowlist
//!   AS-004 — Host validation (DNS rebinding)
//!   AS-005 — Token check (missing + wrong)
//!   AS-008 — Config bootstrap returns frontend_origin
//!   validate_bind_addr — AS-002 unit-level (loopback enforcement)

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use ga_server::lbug_source::fake::FakeDataSource;
use ga_server::{build_app, validate_bind_addr, AppState, JobLauncher, ProjectDataSource, ServerConfig};

/// Test stub — never spawns a real subprocess.
struct NoopLauncher;
impl JobLauncher for NoopLauncher {
    fn spawn_index(
        &self,
        _repo_path: &Path,
        _force: bool,
        _state: std::sync::Arc<std::sync::Mutex<ga_server::jobs::JobState>>,
    ) -> std::io::Result<u32> {
        Ok(0)
    }
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"; // 64 hex
const BACKEND_PORT: u16 = 4317;
const FRONTEND_PORT: u16 = 4318;

fn make_state() -> AppState {
    let cfg = ServerConfig {
        bind: "127.0.0.1:4317".parse().unwrap(),
        cache_root: std::env::temp_dir(),
        token: TOKEN.to_string(),
        allowed_origins: ServerConfig::origins_for_port(FRONTEND_PORT),
        allowed_hosts: ServerConfig::hosts_for_port(BACKEND_PORT),
        frontend_origin: format!("http://localhost:{}", FRONTEND_PORT),
    };
    let data: Arc<dyn ProjectDataSource> = Arc::new(FakeDataSource::new());
    let wd: Arc<dyn ga_server::watcher::WatcherDriver> =
        Arc::new(ga_server::watcher::fake::FakeWatcherDriver::new());
    AppState::new(cfg, Arc::new(NoopLauncher), data, wd)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ---------- AS-001 ----------

#[tokio::test]
async fn as001_health_is_open_no_token() {
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn as001_protected_route_with_origin_and_host_passes() {
    // Token check removed 2026-05-17 — Origin + Host validation is the
    // gate. GET with valid Origin + Host succeeds.
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Origin", format!("http://localhost:{}", FRONTEND_PORT))
                .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------- AS-002 ----------

#[test]
fn as002_validate_bind_rejects_non_loopback() {
    let bad: SocketAddr = "0.0.0.0:4317".parse().unwrap();
    let err = validate_bind_addr(&bad).unwrap_err();
    assert!(err.contains("127.0.0.1"));
}

#[test]
fn as002_validate_bind_accepts_loopback_v4_and_v6() {
    let v4: SocketAddr = "127.0.0.1:4317".parse().unwrap();
    let v6: SocketAddr = "[::1]:4317".parse().unwrap();
    assert!(validate_bind_addr(&v4).is_ok());
    assert!(validate_bind_addr(&v6).is_ok());
}

// ---------- AS-003 ----------

#[tokio::test]
async fn as003_bad_origin_rejected() {
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Origin", "http://evil.com")
                .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "bad_origin");
}

#[tokio::test]
async fn as003_missing_origin_on_get_is_allowed() {
    // Browsers don't send Origin on same-origin GET. X-GA-Token still gates.
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn as003_missing_origin_on_post_rejected() {
    // State-changing methods still require Origin.
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/config")
                .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "bad_origin");
}

// ---------- AS-004 ----------

#[tokio::test]
async fn as004_bad_host_rejected_421() {
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Origin", format!("http://localhost:{}", FRONTEND_PORT))
                .header("Host", "attacker.example.com")
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::MISDIRECTED_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "bad_host");
}

// AS-005 (token check) retired 2026-05-17 — see Spec D Change Log.
// Old test deleted; Origin/Host gates above cover the remaining
// CSRF + DNS rebinding attack surface for the loopback bind.

// ---------- AS-008 ----------

#[tokio::test]
async fn as008_config_endpoint_returns_frontend_origin() {
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Origin", format!("http://localhost:{}", FRONTEND_PORT))
                .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["frontend_origin_expected"], format!("http://localhost:{}", FRONTEND_PORT));
    assert!(body["server_version"].as_str().is_some());
}

// ---------- Defense-in-depth: alt loopback origin accepted ----------

#[tokio::test]
async fn loopback_127_origin_also_accepted() {
    let app = build_app(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .header("Origin", format!("http://127.0.0.1:{}", FRONTEND_PORT))
                .header("Host", format!("localhost:{}", BACKEND_PORT))
                .header("X-GA-Token", TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
