//! Spec E — Graph tab search/sidebar/panel HTTP endpoints.
//! Covers AS-001..AS-018 of `docs/specs/ga-ui/ga-ui-graph-search.md`.
//!
//! Backed by FakeDataSource so we exercise handler routing, validation,
//! and serialization without spinning up lbug.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use ga_server::data::{
    LayerEntry, LayerSymbolsResponse, ParamSlotDto, ProjectDataSource, SymbolDetail, SymbolHit,
    SymbolSearchResponse,
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
    let dir = cache_root.join(format!("fixture-{slug}"));
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
        frontend_origin: format!("http://localhost:{FRONTEND_PORT}"),
    };
    let fake: Arc<FakeDataSource> = Arc::new(FakeDataSource::new());
    let data: Arc<dyn ProjectDataSource> = fake.clone();
    let wd: Arc<dyn ga_server::watcher::WatcherDriver> =
        Arc::new(ga_server::watcher::fake::FakeWatcherDriver::new());
    let state = AppState::new(cfg, Arc::new(NoopLauncher), data, wd);
    Harness { _tmp: tmp, cache_root, fake, state }
}

fn auth(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("Origin", format!("http://localhost:{FRONTEND_PORT}"))
        .header("Host", format!("127.0.0.1:{BACKEND_PORT}"))
        .header("X-GA-Token", TOKEN)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ============================================================
// S-001 — Global symbol search endpoint
// ============================================================

#[tokio::test]
async fn as001_symbols_endpoint_returns_hits() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "search01");
    let mut fx = ProjectFixture::default();
    fx.symbol_search.insert(
        "connect".into(),
        SymbolSearchResponse {
            hits: vec![
                SymbolHit {
                    id: "src/m.py::ConnectingFailedEventArgs:1".into(),
                    name: "ConnectingFailedEventArgs".into(),
                    kind: "function".into(),
                    file: "src/m.py".into(),
                    line: 1,
                    layer: Some("core".into()),
                },
                SymbolHit {
                    id: "src/m.py::OnConnect:2".into(),
                    name: "OnConnect".into(),
                    kind: "function".into(),
                    file: "src/m.py".into(),
                    line: 2,
                    layer: Some("core".into()),
                },
            ],
            truncated: false,
        },
    );
    h.fake.insert("search01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/search01/symbols?q=connect"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 2);
    assert_eq!(body["hits"][0]["name"], "ConnectingFailedEventArgs");
    assert_eq!(body["hits"][0]["kind"], "function");
    assert_eq!(body["hits"][0]["file"], "src/m.py");
    assert_eq!(body["hits"][0]["line"], 1);
    assert_eq!(body["hits"][0]["layer"], "core");
    assert_eq!(body["truncated"], false);
}

#[tokio::test]
async fn as002_symbols_endpoint_truncated_flag_when_capped() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "search02");
    // Fake returns exactly 50 hits + truncated:true.
    let mut hits = Vec::new();
    for i in 0..50 {
        hits.push(SymbolHit {
            id: format!("f.py::get_{i}:1"),
            name: format!("get_{i}"),
            kind: "function".into(),
            file: "f.py".into(),
            line: 1,
            layer: None,
        });
    }
    let mut fx = ProjectFixture::default();
    fx.symbol_search.insert(
        "get".into(),
        SymbolSearchResponse { hits, truncated: true },
    );
    h.fake.insert("search02", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/search02/symbols?q=get&limit=50"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 50);
    assert_eq!(body["truncated"], true);
}

#[tokio::test]
async fn as003_symbols_endpoint_bad_pattern_returns_400() {
    // q=foo() → is_safe_ident reject → 400 bad_pattern.
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "search03");
    h.fake.insert("search03", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .uri("/api/projects/search03/symbols?q=foo%28%29"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"], "bad_pattern");
}

#[tokio::test]
async fn as001_symbols_endpoint_empty_query_returns_400() {
    // q empty → is_safe_ident rejects (empty) → 400 bad_pattern. Frontend
    // is supposed to short-circuit before calling, but the backend MUST
    // not crash and MUST not return all 114k symbols.
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "search04");
    h.fake.insert("search04", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/search04/symbols?q="))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn as005_symbols_endpoint_no_match_returns_empty_hits() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "search05");
    let mut fx = ProjectFixture::default();
    fx.symbol_search.insert(
        "zzznonexistentzzz".into(),
        SymbolSearchResponse {
            hits: vec![],
            truncated: false,
        },
    );
    h.fake.insert("search05", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .uri("/api/projects/search05/symbols?q=zzznonexistentzzz"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["hits"].as_array().unwrap().len(), 0);
    assert_eq!(body["truncated"], false);
}

// ============================================================
// S-002 — Layers + per-layer symbols
// ============================================================

#[tokio::test]
async fn as006_layers_endpoint_returns_modules_sorted_desc() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "layers01");
    let mut fx = ProjectFixture::default();
    fx.layers = Some(ga_server::data::LayersResponse {
        layers: vec![
            LayerEntry { name: "ga-query".into(), symbol_count: 800 },
            LayerEntry { name: "ga-index".into(), symbol_count: 600 },
            LayerEntry { name: "ga-server".into(), symbol_count: 400 },
        ],
        degraded: false,
    });
    h.fake.insert("layers01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/layers01/layers"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let layers = body["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0]["name"], "ga-query");
    assert_eq!(layers[0]["symbol_count"], 800);
    assert_eq!(layers[1]["name"], "ga-index");
    assert_eq!(layers[2]["name"], "ga-server");
    assert_eq!(body["degraded"], false);
}

#[tokio::test]
async fn as007_layers_endpoint_degrades_when_architecture_fails() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "layers02");
    let mut fx = ProjectFixture::default();
    fx.layers = Some(ga_server::data::LayersResponse {
        layers: vec![],
        degraded: true,
    });
    h.fake.insert("layers02", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/layers02/layers"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["layers"].as_array().unwrap().len(), 0);
    assert_eq!(body["degraded"], true);
}

#[tokio::test]
async fn as008_layer_symbols_endpoint_returns_symbols_and_ids() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "layers03");
    let mut fx = ProjectFixture::default();
    fx.layer_symbols.insert(
        "ga-query".into(),
        LayerSymbolsResponse {
            symbols: vec![SymbolHit {
                id: "crates/ga-query/src/symbols.rs::symbols:21".into(),
                name: "symbols".into(),
                kind: "function".into(),
                file: "crates/ga-query/src/symbols.rs".into(),
                line: 21,
                layer: Some("ga-query".into()),
            }],
            symbol_ids: vec!["crates/ga-query/src/symbols.rs::symbols:21".into()],
        },
    );
    h.fake.insert("layers03", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .uri("/api/projects/layers03/layers/ga-query/symbols"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(body["symbols"][0]["name"], "symbols");
    assert_eq!(body["symbol_ids"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn layer_symbols_endpoint_unknown_layer_returns_404() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "layers04");
    h.fake.insert("layers04", ProjectFixture::default());

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder()
                .uri("/api/projects/layers04/layers/nonexistent/symbols"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn layer_name_with_unsafe_chars_returns_400() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "layers05");
    h.fake.insert("layers05", ProjectFixture::default());

    let app = build_app(h.state);
    // "../" segment → path traversal attempt.
    let resp = app
        .oneshot(
            auth(Request::builder()
                .uri("/api/projects/layers05/layers/..%2Fetc/symbols"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================
// S-003 — Right panel — labeled sections (data assembly)
// ============================================================

fn detail_full() -> SymbolDetail {
    SymbolDetail {
        id: "src/r.rs::register:147".into(),
        name: "register".into(),
        kind: "function".into(),
        file: "src/r.rs".into(),
        line: 147,
        line_end: Some(173),
        qualified_name: Some("Router::register".into()),
        rendered_signature: "register(self, path: &str) -> Router".into(),
        layer: Some("router".into()),
        loc: Some(27),
        doc_summary: Some("Register a route handler for the given path.".into()),
        has_doc: true,
        is_async: true,
        is_abstract: false,
        is_static: false,
        is_override: false,
        confidence: 0.95,
        is_dead_code: false,
        is_hub: true,
        tested: true,
        caller_count: 3,
        callee_count: 5,
        importer_count: 2,
        impact_edge_count: 8,
        params: Some(vec![
            ParamSlotDto { name: "self".into(), type_: "&mut Self".into(), default_value: String::new() },
            ParamSlotDto { name: "path".into(), type_: "&str".into(), default_value: String::new() },
        ]),
    }
}

#[tokio::test]
async fn as011_symbol_detail_identity_includes_layer_and_badges() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail01");
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("register".into(), detail_full());
    h.fake.insert("detail01", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail01/symbol/register"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    // IDENTITY fields
    assert_eq!(body["name"], "register");
    assert_eq!(body["kind"], "function");
    assert_eq!(body["layer"], "router");
    assert_eq!(body["file"], "src/r.rs");
    assert_eq!(body["line"], 147);
    // Boolean badges
    assert_eq!(body["is_async"], true);
    assert_eq!(body["is_abstract"], false);
    assert_eq!(body["is_static"], false);
    assert_eq!(body["is_override"], false);
}

#[tokio::test]
async fn as012_symbol_detail_relationships_counts_and_impact() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail02");
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("register".into(), detail_full());
    h.fake.insert("detail02", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail02/symbol/register"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["caller_count"], 3);
    assert_eq!(body["callee_count"], 5);
    assert_eq!(body["importer_count"], 2);
    assert_eq!(body["impact_edge_count"], 8);
}

#[tokio::test]
async fn as013_symbol_detail_quality_fields() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail03");
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("register".into(), detail_full());
    h.fake.insert("detail03", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail03/symbol/register"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert_eq!(body["tested"], true);
    assert_eq!(body["loc"], 27);
    assert_eq!(
        body["doc_summary"],
        "Register a route handler for the given path."
    );
    assert_eq!(body["has_doc"], true);
    assert_eq!(body["confidence"], 0.95);
    assert_eq!(body["is_dead_code"], false);
    assert_eq!(body["is_hub"], true);
}

#[tokio::test]
async fn as014_symbol_detail_no_relations_counts_zero() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail04");
    let mut sd = detail_full();
    sd.caller_count = 0;
    sd.callee_count = 0;
    sd.importer_count = 0;
    sd.impact_edge_count = 0;
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("leaf".into(), sd);
    h.fake.insert("detail04", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail04/symbol/leaf"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["caller_count"], 0);
    assert_eq!(body["callee_count"], 0);
    assert_eq!(body["importer_count"], 0);
    assert_eq!(body["impact_edge_count"], 0);
}

#[tokio::test]
async fn as016_symbol_detail_params_decoded() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail05");
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("register".into(), detail_full());
    h.fake.insert("detail05", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail05/symbol/register"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    let params = body["params"].as_array().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0]["name"], "self");
    assert_eq!(params[0]["type"], "&mut Self");
    assert_eq!(params[1]["name"], "path");
    assert_eq!(params[1]["type"], "&str");
}

#[tokio::test]
async fn as017_symbol_detail_params_degrade_when_decoder_off() {
    // params = None on the wire → JSON null. Frontend renders "—".
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail06");
    let mut sd = detail_full();
    sd.params = None;
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("register".into(), sd);
    h.fake.insert("detail06", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail06/symbol/register"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["params"].is_null(), "expected null, got {:?}", body["params"]);
    // Signature still renders (existing rendered_signature).
    assert_eq!(body["rendered_signature"], "register(self, path: &str) -> Router");
}

#[tokio::test]
async fn as018_symbol_detail_arity_zero_params_empty_vec() {
    let h = make_harness();
    seed_complete_cache(&h.cache_root, "detail07");
    let mut sd = detail_full();
    sd.name = "now".into();
    sd.rendered_signature = "now() -> Instant".into();
    sd.params = Some(vec![]);
    let mut fx = ProjectFixture::default();
    fx.symbol_detail.insert("now".into(), sd);
    h.fake.insert("detail07", fx);

    let app = build_app(h.state);
    let resp = app
        .oneshot(
            auth(Request::builder().uri("/api/projects/detail07/symbol/now"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(resp).await;
    assert!(body["params"].is_array());
    assert_eq!(body["params"].as_array().unwrap().len(), 0);
}
