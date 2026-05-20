//! Integration tests for Spec A S-005 — reindex polling + cancel +
//! recovery.
//!
//! Subprocess monitoring loop (the tokio task that ingests stdout into
//! JobState) is deferred per `.build-checklist` S-005-INFRA. Tests here
//! exercise the state machine + handler contract by manipulating
//! `JobRegistry` + `JobState` directly through the cloned `Arc`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use ga_server::data::ProjectDataSource;
use ga_server::jobs::{JobState, JobStatus};
use ga_server::lbug_source::fake::FakeDataSource;
use ga_server::{build_app, AppState, JobLauncher, ServerConfig};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BACKEND_PORT: u16 = 4317;
const FRONTEND_PORT: u16 = 4318;

#[derive(Default)]
struct RecordingLauncher {
    calls: Mutex<Vec<(PathBuf, bool)>>,
    next_pid: Mutex<Option<u32>>,
}
impl RecordingLauncher {
    fn with_pid(pid: u32) -> Self {
        Self {
            next_pid: Mutex::new(Some(pid)),
            ..Self::default()
        }
    }
}
impl JobLauncher for RecordingLauncher {
    fn spawn_index(
        &self,
        repo_path: &Path,
        force: bool,
        _state: Arc<Mutex<JobState>>,
    ) -> std::io::Result<u32> {
        self.calls
            .lock()
            .unwrap()
            .push((repo_path.to_path_buf(), force));
        Ok(self.next_pid.lock().unwrap().unwrap_or(42))
    }
}

struct Harness {
    _tmp: TempDir,
    cache_root: PathBuf,
    launcher: Arc<RecordingLauncher>,
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

fn make_harness_with_launcher(launcher: Arc<RecordingLauncher>) -> Harness {
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
    let wd: Arc<dyn ga_server::watcher::WatcherDriver> =
        Arc::new(ga_server::watcher::fake::FakeWatcherDriver::new());
    let state = AppState::new(cfg, launcher.clone(), data, wd);
    Harness {
        _tmp: tmp,
        cache_root,
        launcher,
        state,
    }
}

fn make_harness() -> Harness {
    make_harness_with_launcher(Arc::new(RecordingLauncher::with_pid(42)))
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

// ============== AS-042 — happy path POST + status Running ==============

#[tokio::test]
async fn as042_post_reindex_returns_202_and_status_initially_running() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "happy001", repo.path());

    let app = build_app(h.state.clone());
    // 1. POST /reindex
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/happy001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    let job_id = body["job_id"].as_str().unwrap().to_string();

    // 2. GET status
    let resp = app
        .oneshot(
            auth(Request::builder().uri(format!(
                "/api/projects/happy001/reindex/{}/status",
                job_id
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["state"], "Running");
    assert_eq!(body["job_id"], job_id);
    assert_eq!(body["slug"], "happy001");

    // Launcher must have been called exactly once with force=true.
    let calls = h.launcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1, "S-005 always reindexes force=true");
}

// ============== AS-042 — state transitions Running → Done ==============

#[tokio::test]
async fn as042_state_can_transition_to_done() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "donesim1", repo.path());

    let app = build_app(h.state.clone());
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/donesim1/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let job_id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    // Simulate monitor loop completing — set state to Done.
    {
        let handle = h.state.jobs.lookup_by_id(&job_id).unwrap();
        let mut st = handle.state.lock().unwrap();
        st.status = JobStatus::Done;
        st.percent = 100.0;
        st.files_done = 312;
        st.files_total = 312;
        st.duration_ms = 12_450;
    }

    let resp = app
        .oneshot(
            auth(Request::builder().uri(format!(
                "/api/projects/donesim1/reindex/{}/status",
                job_id
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["state"], "Done");
    assert_eq!(body["percent"], 100.0);
    assert_eq!(body["files_done"], 312);
    assert_eq!(body["duration_ms"], 12_450);
}

// ============== AS-043 — 409 conflict if running ==============

#[tokio::test]
async fn as043_second_post_returns_409_with_same_job_id() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "conflict", repo.path());

    let app = build_app(h.state);

    // First POST.
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/conflict/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let first_job_id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    // Second POST while first still active.
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/conflict/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "reindex_in_progress");
    assert_eq!(body["job_id"], first_job_id);
}

// ============== AS-044 — parallel different slugs ==============

#[tokio::test]
async fn as044_different_slugs_reindex_in_parallel() {
    let h = make_harness();
    let r1 = tempdir().unwrap();
    let r2 = tempdir().unwrap();
    seed_cache(&h.cache_root, "par00001", r1.path());
    seed_cache(&h.cache_root, "par00002", r2.path());

    let app = build_app(h.state);
    let r1 = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/par00001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let r2 = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/par00002/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);
    assert_eq!(r2.status(), StatusCode::ACCEPTED);
    let id1 = body_json(r1).await["job_id"].as_str().unwrap().to_string();
    let id2 = body_json(r2).await["job_id"].as_str().unwrap().to_string();
    assert_ne!(id1, id2);
}

// ============== AS-045 — job_id stable across "browser refresh" ==============

#[tokio::test]
async fn as045_job_id_resolves_via_lookup_by_id_after_simulated_refresh() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "refresh1", repo.path());

    let app = build_app(h.state.clone());
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/refresh1/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let job_id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    // Simulate refresh — issue a new GET with just the job_id (no
    // in-memory client state). The server must still find the handle.
    let resp = app
        .oneshot(
            auth(Request::builder().uri(format!(
                "/api/projects/refresh1/reindex/{}/status",
                job_id
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["job_id"], job_id);
    assert_eq!(body["state"], "Running");
}

// ============== AS-046 — DELETE cancels + marks cache corrupt ==============

#[tokio::test]
async fn as046_cancel_marks_state_cancelled_and_cache_corrupt() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "cancel01", repo.path());

    let app = build_app(h.state.clone());
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/cancel01/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let job_id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    // Hold a reference to the state Arc so we can inspect after cancel
    // (cancel removes the slug→handle binding from the registry).
    let handle = h.state.jobs.lookup_by_id(&job_id).unwrap();

    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/cancel01/reindex/{}", job_id)))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body = body_json(resp).await;
    assert_eq!(body["state"], "Cancelled");

    // Verify shared JobState transitioned (we still hold the Arc).
    let snap = handle.state.lock().unwrap().clone();
    assert_eq!(snap.status, JobStatus::Cancelled);

    // Cache must now be marked Corrupt per Spec A AS-046 / A-C8.
    assert_eq!(
        ga_server::cache_state::lookup_cache_state(&h.cache_root, "cancel01"),
        ga_server::cache_state::CacheState::Corrupt
    );
}

#[tokio::test]
async fn cancel_unknown_job_id_returns_404() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("DELETE")
                .uri("/api/projects/any/reindex/nope"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"], "job_not_found");
}

// ============== AS-047 — Error state surfaces error + log_tail ==============

#[tokio::test]
async fn as047_error_state_includes_error_and_log_tail() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "errsim01", repo.path());

    let app = build_app(h.state.clone());
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/errsim01/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let job_id = body_json(resp).await["job_id"].as_str().unwrap().to_string();

    {
        let handle = h.state.jobs.lookup_by_id(&job_id).unwrap();
        let mut st = handle.state.lock().unwrap();
        st.status = JobStatus::Error;
        st.error = Some("parser crash at src/foo.rs".into());
        st.log_tail = vec![
            "[ok] begin reindex".into(),
            "[err] parser crash at src/foo.rs".into(),
        ];
    }

    let resp = app
        .oneshot(
            auth(Request::builder().uri(format!(
                "/api/projects/errsim01/reindex/{}/status",
                job_id
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["state"], "Error");
    assert_eq!(body["error"], "parser crash at src/foo.rs");
    assert_eq!(body["log_tail"].as_array().unwrap().len(), 2);
}

// ============== AS-048 — argv hardening (Command builder shape) ==============

/// The launcher trait contract — `spawn_index(repo_path, force)` — guarantees
/// argv form. We re-verify the production `SubprocessLauncher` (the only
/// real impl) uses `Command::new(absolute_ga).arg(path)`. The S-002 test
/// `as020_argv_literal_no_shell_expansion` already proves the path with
/// shell metacharacters survives untouched; this one asserts the launcher
/// receives the canonical path verbatim during a reindex spawn.
#[tokio::test]
async fn as048_spawn_call_receives_literal_canonical_path() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "argv0001", repo.path());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/argv0001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let calls = h.launcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let canonical = repo.path().canonicalize().unwrap();
    assert_eq!(calls[0].0, canonical);
    assert!(calls[0].1, "force=true on POST /reindex");
}

// ============== AS-049 — recovery scan dead PID → cleanup + mark corrupt ==============

#[tokio::test]
async fn as049_recovery_dead_pid_cleans_up_and_marks_corrupt() {
    use ga_server::recovery::{
        apply_cleanup, find_cache_dir, scan_orphan_pids, RecoveryAction, ReindexPidFile,
    };
    let h = make_harness();
    let repo = tempdir().unwrap();
    let cache_dir = seed_cache(&h.cache_root, "orph0001", repo.path());

    // Pre-seed a pidfile with a guaranteed-dead PID.
    ga_server::recovery::write_pid_file(
        &cache_dir,
        &ReindexPidFile {
            pid: 999_999_999,
            job_id: "old-job".into(),
            slug: "orph0001".into(),
            started_at_unix: 1,
        },
    )
    .unwrap();

    let actions = scan_orphan_pids(&h.cache_root, |_| false);
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        RecoveryAction::Cleanup { slug, cache_dir } => {
            assert_eq!(slug, "orph0001");
            apply_cleanup(cache_dir).unwrap();
        }
        other => panic!("expected Cleanup, got {:?}", other),
    }

    // After cleanup: pidfile gone + cache Corrupt.
    let found = find_cache_dir(&h.cache_root, "orph0001").unwrap();
    assert!(!found.join(".reindex.pid").exists());
    assert_eq!(
        ga_server::cache_state::lookup_cache_state(&h.cache_root, "orph0001"),
        ga_server::cache_state::CacheState::Corrupt
    );
}

#[tokio::test]
async fn as049_recovery_alive_pid_yields_adopt_action() {
    use ga_server::recovery::{scan_orphan_pids, RecoveryAction, ReindexPidFile};
    let h = make_harness();
    let repo = tempdir().unwrap();
    let cache_dir = seed_cache(&h.cache_root, "liveorp1", repo.path());

    ga_server::recovery::write_pid_file(
        &cache_dir,
        &ReindexPidFile {
            pid: 42,
            job_id: "alive-job".into(),
            slug: "liveorp1".into(),
            started_at_unix: 1,
        },
    )
    .unwrap();

    let actions = scan_orphan_pids(&h.cache_root, |_| true);
    assert!(matches!(&actions[0], RecoveryAction::Adopt(pf) if pf.slug == "liveorp1"));
}

// ============== AS-056 — confirm_token TTL expired (slow, #[ignore]) ==============

/// Wall-clock test. Run via `cargo test -p ga-server -- --ignored`.
#[tokio::test]
#[ignore = "slow — sleeps 31s to exceed ConfirmTokens CONFIRM_TTL"]
async fn as056_confirm_token_expired_returns_403() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    seed_cache(&h.cache_root, "ttl00001", repo.path());

    let app = build_app(h.state);

    // Step 1: issue confirm_token.
    let resp = app
        .clone()
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/ttl00001/delete-intent"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let token = body_json(resp).await["confirm_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Step 2: wait past TTL (30s).
    tokio::time::sleep(std::time::Duration::from_secs(31)).await;

    // Step 3: DELETE with now-expired token.
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/ttl00001?confirm={}", token)))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"], "confirm_token_expired");
}

// ============== Stale-Building recovery — metadata says building but no live job ==============
//
// Regression: previous reindex died without flipping metadata.json
// `index_state` back to complete/corrupt; the lookup probe keeps
// reporting Building forever, and POST /reindex was rejected with 409.
// Fix: when the in-process JobRegistry has no live job for the slug,
// treat the disk flag as stale → cleanup + proceed.

#[tokio::test]
async fn stale_building_without_live_job_is_cleared_and_reindex_proceeds() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let cache_dir = seed_cache(&h.cache_root, "stale001", repo.path());

    // Mutate metadata.json: index_state → "building" (mimic crashed reindex).
    let md_path = cache_dir.join("metadata.json");
    let mut md: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&md_path).unwrap()).unwrap();
    md["index_state"] = serde_json::json!("building");
    std::fs::write(&md_path, serde_json::to_vec(&md).unwrap()).unwrap();
    assert_eq!(
        ga_server::cache_state::lookup_cache_state(&h.cache_root, "stale001"),
        ga_server::cache_state::CacheState::Building
    );

    // No pidfile, no in-registry job — pure stale flag on disk.
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/stale001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "stale Building flag should be cleaned up, not block start"
    );

    // Regression: cleanup must NOT stamp metadata `index_state: "corrupt"`
    // — that string is not a valid `ga_core::IndexState` variant, so the
    // freshly-spawned reindex subprocess would fail to open the cache
    // ("unknown variant `corrupt`, expected `building` or `complete`").
    let md: serde_json::Value =
        serde_json::from_slice(&std::fs::read(cache_dir.join("metadata.json")).unwrap()).unwrap();
    let state_str = md["index_state"].as_str().unwrap();
    assert!(
        state_str == "building" || state_str == "complete",
        "metadata.index_state must stay ga_core-parseable, got {:?}",
        state_str
    );
}

// ============== H-1: stale-Building with LIVE pidfile must 409, not steal ==============
//
// Regression: post-restart, a `ga reindex` grandchild may still be alive
// while ga-server's JobRegistry is empty (recovery scan not wired at
// startup yet). The earlier cleanup unlinked the pidfile + proceeded to
// spawn a second writer on the same cache → flock race + potential
// corruption. Fix: peek at pidfile, if PID still alive, return 409 with
// the existing job_id (matching JobInsertResult::Existing shape).

#[tokio::test]
async fn stale_building_with_live_pidfile_is_not_unlinked() {
    let h = make_harness();
    let repo = tempdir().unwrap();
    let cache_dir = seed_cache(&h.cache_root, "alive001", repo.path());

    // Pre-seed metadata as Building (mimic mid-reindex).
    let md_path = cache_dir.join("metadata.json");
    let mut md: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&md_path).unwrap()).unwrap();
    md["index_state"] = serde_json::json!("building");
    std::fs::write(&md_path, serde_json::to_vec(&md).unwrap()).unwrap();

    // Pre-seed a pidfile pointing at OUR process PID — guaranteed alive.
    let my_pid = std::process::id();
    ga_server::recovery::write_pid_file(
        &cache_dir,
        &ga_server::recovery::ReindexPidFile {
            pid: my_pid,
            job_id: "orphan-job".into(),
            slug: "alive001".into(),
            started_at_unix: 1,
        },
    )
    .unwrap();

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/alive001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "live orphan pidfile must block start, not be silently unlinked"
    );
    let body = body_json(resp).await;
    assert_eq!(body["error"], "reindex_in_progress");
    assert_eq!(body["job_id"], "orphan-job");
    // Pidfile must still exist — not unlinked.
    assert!(cache_dir.join(".reindex.pid").exists());
}

// ============== Path safety regression — repo path with missing dir ==============

#[tokio::test]
async fn post_reindex_with_orphan_repo_root_returns_400() {
    let h = make_harness();
    seed_cache(&h.cache_root, "ghost001", Path::new("/tmp/__never_exists_xyzzy__"));
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .method("POST")
                .uri("/api/projects/ghost001/reindex"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
