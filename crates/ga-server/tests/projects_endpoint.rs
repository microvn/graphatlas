//! Integration tests for Spec A S-002 — projects registry + 2-step DELETE.
//!
//! Each test names the AS it covers. Subprocess spawn is stubbed via
//! `RecordingLauncher` — AS-020 verifies argv literal-arg semantics by
//! inspecting the recorded calls instead of running a real `ga index`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use ga_server::lbug_source::fake::FakeDataSource;
use ga_server::{build_app, AppState, JobLauncher, ProjectDataSource, ServerConfig};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BACKEND_PORT: u16 = 4317;
const FRONTEND_PORT: u16 = 4318;

#[derive(Default)]
struct RecordingLauncher {
    calls: Mutex<Vec<(PathBuf, bool)>>,
}

impl JobLauncher for RecordingLauncher {
    fn spawn_index(
        &self,
        repo_path: &Path,
        force: bool,
        _state: Arc<std::sync::Mutex<ga_server::jobs::JobState>>,
    ) -> std::io::Result<u32> {
        self.calls
            .lock()
            .unwrap()
            .push((repo_path.to_path_buf(), force));
        Ok(42) // dummy pid
    }
}

struct Harness {
    _cache_dir: TempDir,
    cache_root: PathBuf,
    launcher: Arc<RecordingLauncher>,
    state: AppState,
}

fn make_harness() -> Harness {
    let cache_dir = tempdir().unwrap();
    let cache_root = cache_dir.path().to_path_buf();
    let cfg = ServerConfig {
        bind: "127.0.0.1:4317".parse().unwrap(),
        cache_root: cache_root.clone(),
        token: TOKEN.to_string(),
        allowed_origins: ServerConfig::origins_for_port(FRONTEND_PORT),
        allowed_hosts: ServerConfig::hosts_for_port(BACKEND_PORT),
        frontend_origin: format!("http://localhost:{}", FRONTEND_PORT),
    };
    let launcher = Arc::new(RecordingLauncher::default());
    let data: Arc<dyn ProjectDataSource> = Arc::new(FakeDataSource::new());
    let wd: Arc<dyn ga_server::watcher::WatcherDriver> =
        Arc::new(ga_server::watcher::fake::FakeWatcherDriver::new());
    let state = AppState::new(cfg, launcher.clone(), data, wd);
    Harness {
        _cache_dir: cache_dir,
        cache_root,
        launcher,
        state,
    }
}

fn auth_headers(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("Origin", format!("http://localhost:{}", FRONTEND_PORT))
        .header("Host", format!("127.0.0.1:{}", BACKEND_PORT))
        .header("X-GA-Token", TOKEN)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// Build a valid pre-existing cache dir with metadata.json matching
/// the format `ga_index::list::list_caches` expects.
fn seed_cache(cache_root: &Path, name: &str, repo_root: &Path) -> PathBuf {
    // Cache dir naming: `<name>-<6hex>` per Foundation-C12. We use a
    // fixed 6-hex suffix for deterministic slug assertions.
    let suffix = "abcdef";
    let dir_name = format!("{}-{}", name, suffix);
    let dir = cache_root.join(&dir_name);
    std::fs::create_dir_all(&dir).unwrap();
    let metadata = serde_json::json!({
        "schema_version": 9,
        "indexed_at": 1715712000u64,
        "committed_at": 1715712000u64,
        "repo_root": repo_root.display().to_string(),
        "index_state": "complete",
        "index_generation": "test-generation",
        "indexed_root_hash": "",
        "graph_generation": 1,
        "cache_lang_set": []
    });
    std::fs::write(
        dir.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    dir
}

// ============================================================
// AS-010 — GET /api/projects happy path
// ============================================================

#[tokio::test]
async fn as010_list_projects_returns_rows() {
    let h = make_harness();
    let repo1 = tempdir().unwrap();
    let repo2 = tempdir().unwrap();
    seed_cache(&h.cache_root, "repo-one", repo1.path());
    seed_cache(&h.cache_root, "repo-two", repo2.path());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(Request::builder().uri("/api/projects"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let arr = body.as_array().expect("response must be array");
    assert_eq!(arr.len(), 2);
    let first = &arr[0];
    assert!(first["slug"].as_str().is_some());
    assert_eq!(first["index_state"], "Fresh");
    assert!(first["index_counts"].is_null(), "pre-migration cache → index_counts: null");
    assert!(first["health"].is_null());
}

// ============================================================
// AS-011 — corrupt cache → X-GA-Corrupt-Count header
// ============================================================

#[tokio::test]
async fn as011_corrupt_cache_emits_header() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "good", repo.path());
    // Corrupt: metadata.json exists but invalid JSON.
    let bad = h.cache_root.join("bad-cache-999999");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("metadata.json"), b"{ not json").unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(Request::builder().uri("/api/projects"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let corrupt_header = resp
        .headers()
        .get("x-ga-corrupt-count")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(corrupt_header, "1");
}

// ============================================================
// AS-012 — empty registry → []
// ============================================================

#[tokio::test]
async fn as012_empty_registry_returns_empty_array() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(Request::builder().uri("/api/projects"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

// ============================================================
// AS-013 — POST add new path (mode=index) → 202 + spawn
// ============================================================

#[tokio::test]
async fn as013_add_new_path_spawns_index() {
    let h = make_harness();
    let repo = tempdir().unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({
                    "path": repo.path().display().to_string(),
                    "mode": "index",
                })
                .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    assert!(body["job_id"].as_str().is_some());
    assert_eq!(body["mode"], "index");

    let calls = h.launcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "spawn_index called exactly once");
    assert!(!calls[0].1, "force=false for new path");
}

// ============================================================
// AS-014 — Attach mode requires existing cache
// ============================================================

#[tokio::test]
async fn as014_attach_with_existing_cache_no_spawn() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    // Pre-seed a cache whose slug matches the canonical path. Slug =
    // blake3 over canonical_path's lossy str, first 8 hex chars.
    let canonical = repo.path().canonicalize().unwrap();
    let slug = ga_server::handlers::projects::slug_for(&canonical);
    let dir = h.cache_root.join(format!("attach-{}", slug));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("metadata.json"),
        serde_json::json!({
            "schema_version": 9,
            "indexed_at": 1715712000u64,
            "committed_at": 1715712000u64,
            "repo_root": canonical.display().to_string(),
            "index_state": "complete",
            "index_generation": "g",
            "indexed_root_hash": "",
            "graph_generation": 1,
            "cache_lang_set": []
        })
        .to_string(),
    )
    .unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({
                    "path": canonical.display().to_string(),
                    "mode": "attach",
                })
                .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["job_id"].is_null());
    assert_eq!(body["mode"], "attach");
    assert_eq!(h.launcher.calls.lock().unwrap().len(), 0, "attach must not spawn");
}

// ============================================================
// AS-015 — mode=index on existing cache → spawn with --force
// ============================================================

#[tokio::test]
async fn as015_index_existing_cache_forces_reindex() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let canonical = repo.path().canonicalize().unwrap();
    let slug = ga_server::handlers::projects::slug_for(&canonical);
    let dir = h.cache_root.join(format!("force-{}", slug));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("metadata.json"),
        serde_json::json!({
            "schema_version": 9,
            "indexed_at": 1715712000u64,
            "committed_at": 1715712000u64,
            "repo_root": canonical.display().to_string(),
            "index_state": "complete",
            "index_generation": "g",
            "indexed_root_hash": "",
            "graph_generation": 1,
            "cache_lang_set": []
        })
        .to_string(),
    )
    .unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({
                    "path": canonical.display().to_string(),
                    "mode": "index",
                })
                .to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let calls = h.launcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1, "force=true for existing cache + mode=index");
}

// ============================================================
// AS-016 / AS-017 / AS-018 / AS-019 — path safety rejections
// ============================================================

#[tokio::test]
async fn as016_path_not_found() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": "/tmp/__nope__/__xyz__"}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "path_not_found");
}

#[tokio::test]
async fn as017_path_not_directory() {
    let h = make_harness();
    let f = tempdir().unwrap();
    let file = f.path().join("a.txt");
    std::fs::write(&file, b"x").unwrap();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": file.display().to_string()}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "path_not_directory");
}

#[tokio::test]
async fn as018_path_with_dotdot_rejected() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": "/tmp/../etc"}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "path_unsafe");
}

#[tokio::test]
async fn as018_path_into_cache_root_rejected() {
    let h = make_harness();
    let inside = h.cache_root.join("victim");
    std::fs::create_dir_all(&inside).unwrap();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": inside.display().to_string()}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "path_unsafe");
}

#[tokio::test]
#[cfg(unix)]
async fn as019_external_symlink_rejected() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.path().join("evil")).unwrap();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": repo.path().display().to_string()}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(resp).await["error"],
        "path_contains_external_symlink"
    );
}

// ============================================================
// AS-020 — argv injection literal, no shell exec
// ============================================================

/// Naming a directory with shell metacharacters and verifying the
/// launcher receives them as a single literal argv slot. Real-world
/// equivalent of `Command::new(ga).arg(literal_with_; rm -rf)`.
#[tokio::test]
async fn as020_argv_literal_no_shell_expansion() {
    let h = make_harness();
    let parent = tempdir().unwrap();
    let weird = parent.path().join("evil; rm -rf");
    std::fs::create_dir(&weird).unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("Content-Type", "application/json"),
            )
            .body(Body::from(
                serde_json::json!({"path": weird.display().to_string(), "mode":"index"}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let calls = h.launcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let recorded = &calls[0].0;
    // Critical: the launcher received the FULL literal path as a single
    // arg — no shell splitting on `;`.
    let canonical = weird.canonicalize().unwrap();
    assert_eq!(recorded, &canonical);
    // /tmp/pwned must NOT exist (would mean shell ran the `rm` part).
    assert!(!std::path::Path::new("/tmp/pwned").exists());
}

// ============================================================
// AS-021 — concurrent POST same path → race-safe
// ============================================================

#[tokio::test]
async fn as021_concurrent_post_same_path_only_one_spawn() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let path_str = repo.path().display().to_string();
    let app = build_app(h.state.clone());

    let mut handles = Vec::new();
    for _ in 0..10 {
        let app = app.clone();
        let path_str = path_str.clone();
        handles.push(tokio::spawn(async move {
            app.oneshot(
                auth_headers(
                    Request::builder()
                        .method("POST")
                        .uri("/api/projects")
                        .header("Content-Type", "application/json"),
                )
                .body(Body::from(
                    serde_json::json!({"path": path_str, "mode":"index"}).to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap()
        }));
    }
    let mut accepted = 0;
    let mut conflicts = 0;
    let mut conflict_job_ids = std::collections::HashSet::new();
    let mut accepted_job_id: Option<String> = None;
    for h in handles {
        let resp = h.await.unwrap();
        let status = resp.status();
        let body = body_json(resp).await;
        if status == StatusCode::ACCEPTED {
            accepted += 1;
            accepted_job_id = body["job_id"].as_str().map(String::from);
        } else if status == StatusCode::CONFLICT {
            conflicts += 1;
            if let Some(id) = body["job_id"].as_str() {
                conflict_job_ids.insert(id.to_string());
            }
        } else {
            panic!("unexpected status: {}", status);
        }
    }
    assert_eq!(accepted, 1, "exactly 1 spawn accepted");
    assert_eq!(conflicts + accepted, 10);
    // All conflict responses must echo the same job_id as the accepted one.
    if let Some(accepted_id) = accepted_job_id {
        for cid in &conflict_job_ids {
            assert_eq!(cid, &accepted_id);
        }
    }
    assert_eq!(
        h.launcher.calls.lock().unwrap().len(),
        1,
        "launcher must have been invoked exactly once"
    );
}

// ============================================================
// AS-022 — 2-step DELETE happy path
// ============================================================

#[tokio::test]
async fn as022_two_step_delete_happy() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let cache_dir = seed_cache(&h.cache_root, "victim", repo.path());
    // Slug = trailing piece per scan_projects logic = "abcdef".
    let slug = "abcdef";

    let app = build_app(h.state);

    // Step 1: delete-intent
    let resp = app
        .clone()
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/projects/{}/delete-intent", slug)),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let token = body["confirm_token"].as_str().unwrap().to_string();
    assert!(body["expires_in_secs"].as_u64().unwrap() > 0);

    // Step 2: DELETE with token
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/projects/{}?confirm={}", slug, token)),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(!cache_dir.exists(), "cache dir must be removed");
}

// ============================================================
// AS-023 — DELETE without confirm token → 403
// ============================================================

#[tokio::test]
async fn as023_delete_without_confirm_token_403() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let _cache_dir = seed_cache(&h.cache_root, "x", repo.path());
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/projects/abcdef"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"], "missing_confirm_token");
}

// ============================================================
// AS-024 — confirm_token expired → 403
// ============================================================

#[tokio::test]
async fn as024_invalid_confirm_token_rejected() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "y", repo.path());
    let app = build_app(h.state);

    // Intent issued — we then DELETE with a different token. Same path
    // as "expired" from the validator's POV (Mismatch).
    let resp = app
        .clone()
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects/abcdef/delete-intent"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(
            auth_headers(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/projects/abcdef?confirm=wrong-token"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"], "invalid_confirm_token");
}

// ============================================================
// AS-025 — orphan project (path missing)
// ============================================================

#[tokio::test]
async fn as025_orphan_project_listed_with_orphan_state() {
    let h = make_harness();
    let orphan_path = h.cache_root.join("__never_exists__"); // doesn't exist on disk
    seed_cache(&h.cache_root, "ghost", &orphan_path);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth_headers(Request::builder().uri("/api/projects"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let row = &body.as_array().unwrap()[0];
    assert_eq!(row["index_state"], "Orphan");
}
