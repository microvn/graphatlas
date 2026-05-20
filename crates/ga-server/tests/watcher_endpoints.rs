//! Integration tests for Spec A S-006 — watcher start / stop / status.
//!
//! Real notify-rs spawn is deferred per `.build-checklist` S-006-INFRA.
//! Tests drive the FakeWatcherDriver to assert handler behavior +
//! state-machine transitions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use ga_server::data::ProjectDataSource;
use ga_server::lbug_source::fake::FakeDataSource;
use ga_server::watcher::fake::FakeWatcherDriver;
use ga_server::watcher::{StartOutcome, WatcherDriver};
use ga_server::{build_app, AppState, JobLauncher, ServerConfig};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BACKEND_PORT: u16 = 4317;
const FRONTEND_PORT: u16 = 4318;

struct NoopLauncher;
impl JobLauncher for NoopLauncher {
    fn spawn_index(
        &self,
        _p: &Path,
        _f: bool,
        _s: std::sync::Arc<std::sync::Mutex<ga_server::jobs::JobState>>,
    ) -> std::io::Result<u32> {
        Ok(0)
    }
}

struct Harness {
    _tmp: TempDir,
    cache_root: PathBuf,
    fake_driver: Arc<FakeWatcherDriver>,
    state: AppState,
}

fn seed_cache(cache_root: &Path, slug: &str, repo_root: &Path) -> PathBuf {
    let dir = cache_root.join(format!("fix-{}", slug));
    std::fs::create_dir_all(&dir).unwrap();
    let md = serde_json::json!({
        "schema_version": 5,
        "indexed_at": 1u64,
        "committed_at": 1u64,
        "repo_root": repo_root.display().to_string(),
        "index_state": "complete",
        "index_generation": "g",
        "indexed_root_hash": "",
        "graph_generation": 1,
        "cache_lang_set": []
    });
    std::fs::write(dir.join("metadata.json"), serde_json::to_vec(&md).unwrap()).unwrap();
    dir
}

fn make_harness() -> Harness {
    let tmp = tempdir().unwrap();
    let cache_root = tmp.path().to_path_buf();
    let cfg = ServerConfig {
        bind: "127.0.0.1:4317".parse().unwrap(),
        cache_root: cache_root.clone(),
        token: TOKEN.into(),
        allowed_origins: ServerConfig::origins_for_port(FRONTEND_PORT),
        allowed_hosts: ServerConfig::hosts_for_port(BACKEND_PORT),
        frontend_origin: format!("http://localhost:{}", FRONTEND_PORT),
    };
    let data: Arc<dyn ProjectDataSource> = Arc::new(FakeDataSource::new());
    let fake_driver = Arc::new(FakeWatcherDriver::new());
    let wd: Arc<dyn WatcherDriver> = fake_driver.clone();
    let state = AppState::new(cfg, Arc::new(NoopLauncher), data, wd);
    Harness {
        _tmp: tmp,
        cache_root,
        fake_driver,
        state,
    }
}

fn auth(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("Origin", format!("http://localhost:{}", FRONTEND_PORT))
        .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
        .header("X-GA-Token", TOKEN)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ============== AS-050 — POST start ==============

#[tokio::test]
async fn as050_start_watcher_returns_running() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "wstart01", repo.path());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/wstart01/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"start"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "Running");
    // Native watcher backend name varies by host OS (FSEvents on macOS,
    // inotify on Linux, RDCW on Windows). Accept any non-"poll" string.
    let mode = body["mode"].as_str().unwrap();
    assert!(
        ["fsevents", "inotify", "rdcw", "native"].contains(&mode),
        "unexpected mode {mode}"
    );
    assert!(h.fake_driver.starts().contains(&"wstart01".to_string()));
}

// ============== AS-053 — ENOSPC fallback to poll ==============

#[tokio::test]
async fn as053_enospc_failure_falls_back_to_poll_mode() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "enospc01", repo.path());
    h.fake_driver.set_outcome(
        "enospc01",
        StartOutcome::Failed(
            "inotify_init: ENOSPC: max watches exceeded for user".into(),
        ),
    );

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/enospc01/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"start"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "Running");
    assert_eq!(body["mode"], "poll");
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("Falling back to polling"));
}

#[tokio::test]
async fn explicit_fallback_poll_outcome_is_honored() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "fallbk01", repo.path());
    h.fake_driver.set_outcome(
        "fallbk01",
        StartOutcome::FallbackPoll("driver requested fallback".into()),
    );

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/fallbk01/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"start"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["mode"], "poll");
}

// ============== Errored path — failure that's not ENOSPC ==============

#[tokio::test]
async fn unexpected_start_failure_marks_errored() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "errored1", repo.path());
    h.fake_driver.set_outcome(
        "errored1",
        StartOutcome::Failed("permission denied at .git/HEAD".into()),
    );

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/errored1/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"start"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["status"], "Errored");
    assert!(body["error"].as_str().unwrap_or("").contains("permission denied"));
}

// ============== AS-055 — POST stop ==============

#[tokio::test]
async fn as055_stop_watcher_returns_stopped_and_clears_state() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "wstop001", repo.path());

    let app = build_app(h.state.clone());

    // First start it.
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/wstop001/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"start"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "Running");

    // Pre-seed pending events + dirty_flag so we can verify they reset.
    {
        let entry = h.state.watchers.entry("wstop001");
        let mut guard = entry.lock().unwrap();
        guard.queue_pending = 42;
        guard.dirty_flag = true;
    }

    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/wstop001/watcher")
                .header("Content-Type", "application/json"))
            .body(Body::from(r#"{"action":"stop"}"#))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "Stopped");
    assert_eq!(body["queue_pending"], 0);
    assert_eq!(body["dirty_flag"], false);
    assert!(h.fake_driver.stops().contains(&"wstop001".to_string()));
}

// ============== GET status ==============

#[tokio::test]
async fn get_watcher_status_returns_default_stopped_for_fresh_project() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "wstatus1", repo.path());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/wstatus1/watcher"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "Stopped");
    assert_eq!(body["queue_pending"], 0);
    assert_eq!(body["dirty_flag"], false);
}

#[tokio::test]
async fn watcher_endpoint_returns_404_for_missing_project() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/nope0001/watcher"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"], "project_not_found");
}

// ============== AS-051 — should_trigger_reindex policy ==============
// Pure-logic helper is already unit-tested in watcher.rs; integration
// test asserts the JobRegistry interaction at the boundary.

#[tokio::test]
async fn as051_should_trigger_reindex_skips_when_job_in_flight() {
    use ga_server::watcher::should_trigger_reindex;
    let h = make_harness();
    // No job yet → trigger.
    assert!(should_trigger_reindex(&h.state.jobs, "any"));
    // Insert a job → must skip.
    let _ = h.state.jobs.try_insert("any");
    assert!(!should_trigger_reindex(&h.state.jobs, "any"));
}

// ============== AS-052 — queue cap policy ==============

#[tokio::test]
async fn as052_queue_cap_triggers_dirty_flag_mode() {
    use ga_server::watcher::{should_switch_to_dirty_flag, QUEUE_CAP};
    assert!(!should_switch_to_dirty_flag(QUEUE_CAP));
    assert!(should_switch_to_dirty_flag(QUEUE_CAP + 1));
    assert!(should_switch_to_dirty_flag(100_000));
}

// ============== AS-054 — git op detection ==============

#[tokio::test]
async fn as054_git_rebase_marker_blocks_reindex() {
    use ga_server::watcher::is_git_op_in_progress;
    let repo = tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join(".git/rebase-merge")).unwrap();
    assert!(is_git_op_in_progress(repo.path()));
}

#[tokio::test]
async fn git_index_lock_blocks_reindex() {
    use ga_server::watcher::is_git_op_in_progress;
    let repo = tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join(".git")).unwrap();
    std::fs::write(repo.path().join(".git/index.lock"), b"").unwrap();
    assert!(is_git_op_in_progress(repo.path()));
}
