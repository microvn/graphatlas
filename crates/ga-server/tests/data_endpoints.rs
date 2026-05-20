//! Integration tests for Spec A S-004 — graph / symbol detail /
//! callers / callees / importers / file summary endpoints, plus
//! middleware-level AS-040 (X-GA-Stale) and AS-041 (cache Corrupt 503).
//!
//! Backed by `FakeDataSource` so we can assert handler behavior without
//! spinning up lbug.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use ga_server::data::{
    FileSummary, GraphEdge, GraphNode, GraphResponse, ProjectDataSource, RelationEntry,
    SymbolDetail,
};
use ga_server::lbug_source::fake::{FakeDataSource, ProjectFixture};
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
    fake: Arc<FakeDataSource>,
    state: AppState,
}

fn seed_complete_cache(cache_root: &Path, slug: &str) -> PathBuf {
    let dir = cache_root.join(format!("fixture-{}", slug));
    std::fs::create_dir_all(&dir).unwrap();
    let md = serde_json::json!({
        "schema_version": 5,
        "indexed_at": 1u64,
        "committed_at": 1u64,
        "repo_root": "/tmp/fixture-repo",
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
    let fake: Arc<FakeDataSource> = Arc::new(FakeDataSource::new());
    let data: Arc<dyn ProjectDataSource> = fake.clone();
    let wd: Arc<dyn ga_server::watcher::WatcherDriver> =
        Arc::new(ga_server::watcher::fake::FakeWatcherDriver::new());
    let state = AppState::new(cfg, Arc::new(NoopLauncher), data, wd);
    Harness {
        _tmp: tmp,
        cache_root,
        fake,
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

fn small_graph() -> GraphResponse {
    GraphResponse {
        nodes: (0..8)
            .map(|i| GraphNode {
                id: format!("sym_{}", i),
                name: format!("fn_{}", i),
                kind: "Function".into(),
                file: "src/lib.rs".into(),
                line: (i + 1) as u32,
                line_end: Some((i + 5) as u32),
                degree: 3,
            })
            .collect(),
        edges: (0..7)
            .map(|i| GraphEdge {
                from: format!("sym_{}", i),
                to: format!("sym_{}", i + 1),
                kind: "CALLS".into(),
                line: Some(((i + 1) * 10) as u32),
            })
            .collect(),
        truncated: false,
        total_node_count: 8,
    }
}

fn entries_n(n: u64) -> Vec<RelationEntry> {
    (0..n)
        .map(|i| RelationEntry {
            id: format!("c_{}", i),
            name: format!("caller_{}", i),
            file: format!("src/c{}.rs", i),
            line: (i + 1) as u32,
            kind: "Function".into(),
        })
        .collect()
}

// ============== AS-032 graph small repo ==============

#[tokio::test]
async fn as032_graph_small_repo_returns_full_payload() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "abc12345");
    let mut fx = ProjectFixture::default();
    fx.graph = Some(small_graph());
    h.fake.insert("abc12345", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/abc12345/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["truncated"], false);
    assert_eq!(body["total_node_count"], 8);
    assert_eq!(body["nodes"].as_array().unwrap().len(), 8);
    assert_eq!(body["edges"].as_array().unwrap().len(), 7);
}

// ============== AS-033 large repo capped ==============

#[tokio::test]
async fn as033_graph_large_repo_truncated_flag() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "biggrf12");
    let mut big = small_graph();
    big.nodes.truncate(5000);
    big.truncated = true;
    big.total_node_count = 60_000;
    let mut fx = ProjectFixture::default();
    fx.graph = Some(big);
    h.fake.insert("biggrf12", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/biggrf12/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["truncated"], true);
    assert_eq!(body["total_node_count"], 60_000);
}

// ============== AS-034 focus ego graph ==============

#[tokio::test]
async fn as034_graph_focus_returns_focused_subgraph() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "egofcs99");
    let mut fx = ProjectFixture::default();
    fx.focused_graph.insert(
        "sym_3".into(),
        GraphResponse {
            nodes: vec![GraphNode {
                id: "sym_3".into(),
                name: "router_new".into(),
                kind: "Function".into(),
                file: "src/r.rs".into(),
                line: 10,
                line_end: Some(20),
                degree: 12,
            }],
            edges: vec![],
            truncated: false,
            total_node_count: 1,
        },
    );
    h.fake.insert("egofcs99", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/egofcs99/graph?focus=sym_3&hops=2"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["nodes"][0]["id"], "sym_3");
}

// ============== AS-035 symbol detail rendered_signature ==============

#[tokio::test]
async fn as035_symbol_detail_returns_rendered_signature() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "symdtl01");
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert(
        "sym_42".into(),
        SymbolDetail {
            id: "sym_42".into(),
            name: "new".into(),
            kind: "Function".into(),
            file: "src/routing/router.rs".into(),
            line: 147,
            line_end: Some(160),
            qualified_name: Some("Router::new".into()),
            rendered_signature: "new(self: &mut Self, path: &str) -> Router".into(),
            layer: Some("router".into()),
            ..Default::default()
        },
    );
    h.fake.insert("symdtl01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/symdtl01/symbol/sym_42"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(
        body["rendered_signature"],
        "new(self: &mut Self, path: &str) -> Router"
    );
    assert_eq!(body["qualified_name"], "Router::new");
}

// ============== AS-036 / AS-037 callers pagination ==============

#[tokio::test]
async fn as036_callers_default_limit_50_with_total_and_has_more() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "callpg01");
    let mut fx = ProjectFixture::default();
    fx.callers.insert("hub".into(), entries_n(200));
    h.fake.insert("callpg01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/callpg01/symbol/hub/callers"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["total"], 200);
    assert_eq!(body["has_more"], true);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["limit"], 50);
    assert_eq!(body["entries"].as_array().unwrap().len(), 50);
    assert_eq!(body["entries"][0]["id"], "c_0");
    assert_eq!(body["entries"][49]["id"], "c_49");
}

#[tokio::test]
async fn as037_callers_paginated_offset_50_limit_50() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "callpg02");
    let mut fx = ProjectFixture::default();
    fx.callers.insert("hub".into(), entries_n(200));
    h.fake.insert("callpg02", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(
                Request::builder()
                    .uri("/api/projects/callpg02/symbol/hub/callers?offset=50&limit=50"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["entries"][0]["id"], "c_50");
    assert_eq!(body["entries"][49]["id"], "c_99");
    assert_eq!(body["has_more"], true);
}

#[tokio::test]
async fn callers_last_page_has_more_false() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "callpg03");
    let mut fx = ProjectFixture::default();
    fx.callers.insert("hub".into(), entries_n(75));
    h.fake.insert("callpg03", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(
                Request::builder()
                    .uri("/api/projects/callpg03/symbol/hub/callers?offset=50&limit=50"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["entries"].as_array().unwrap().len(), 25);
    assert_eq!(body["has_more"], false);
}

// ============== AS-038 symbol not found ==============

#[tokio::test]
async fn as038_symbol_not_found_returns_404() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "miss0001");
    h.fake.insert("miss0001", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/miss0001/symbol/nope"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "symbol_not_found");
}

// ============== AS-039 file summary ==============

#[tokio::test]
async fn as039_file_summary_returns_symbols_imports_reverse_imports() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "filsum01");
    let mut fx = ProjectFixture::default();
    fx.file_summary.insert(
        "src/router/mod.rs".into(),
        FileSummary {
            path: "src/router/mod.rs".into(),
            language: Some("rust".into()),
            line_count: Some(412),
            symbols: vec![RelationEntry {
                id: "sym_1".into(),
                name: "Router".into(),
                file: "src/router/mod.rs".into(),
                line: 12,
                kind: "Struct".into(),
            }],
            imports: vec!["src/http.rs".into()],
            reverse_imports: vec!["src/main.rs".into(), "tests/route_test.rs".into()],
        },
    );
    h.fake.insert("filsum01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/filsum01/file?path=src/router/mod.rs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["path"], "src/router/mod.rs");
    assert_eq!(body["language"], "rust");
    assert_eq!(body["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(body["imports"].as_array().unwrap().len(), 1);
    assert_eq!(body["reverse_imports"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn file_summary_not_found_returns_404() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "fnotfd01");
    h.fake.insert("fnotfd01", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/fnotfd01/file?path=ghost.rs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "file_not_found");
}

// ============== AS-040 X-GA-Stale header ==============

#[tokio::test]
async fn as040_query_during_reindex_emits_x_ga_stale_header() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "stale001");
    let mut fx = ProjectFixture::default();
    fx.graph = Some(small_graph());
    h.fake.insert("stale001", fx);

    // Simulate active reindex job by inserting into JobRegistry.
    let _ = h.state.jobs.try_insert("stale001");

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/stale001/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let header = resp
        .headers()
        .get("x-ga-stale")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(header, "reindex-in-progress");
}

#[tokio::test]
async fn no_x_ga_stale_when_no_active_job() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "fresh001");
    let mut fx = ProjectFixture::default();
    fx.graph = Some(small_graph());
    h.fake.insert("fresh001", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/fresh001/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("x-ga-stale").is_none());
}

// ============== AS-041 cache Corrupt 503 ==============

#[tokio::test]
async fn as041_cache_corrupt_returns_503() {
    let h = make_harness();
    // Seed cache dir with index_state: corrupt marker.
    let dir = h.cache_root.join("fixture-corp0001");
    std::fs::create_dir_all(&dir).unwrap();
    let body = serde_json::json!({
        "schema_version": 5,
        "indexed_at": 1u64,
        "committed_at": 1u64,
        "repo_root": "/tmp/x",
        "index_state": "corrupt",
        "index_generation": "g",
        "indexed_root_hash": "",
        "graph_generation": 1,
        "cache_lang_set": []
    });
    std::fs::write(
        dir.join("metadata.json"),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
    h.fake.insert("corp0001", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/corp0001/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "cache_corrupt");
}

#[tokio::test]
async fn missing_cache_returns_404() {
    let h = make_harness();
    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/nope/graph"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "project_not_found");
}

// ============== bound: clamp_limit refuses > MAX_PAGE_SIZE ==============

#[tokio::test]
async fn pagination_clamped_to_max_page_size() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "cap00001");
    let mut fx = ProjectFixture::default();
    fx.callers.insert("hub".into(), entries_n(1000));
    h.fake.insert("cap00001", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/cap00001/symbol/hub/callers?limit=99999"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["limit"], 500); // MAX_PAGE_SIZE
    assert_eq!(body["entries"].as_array().unwrap().len(), 500);
}
