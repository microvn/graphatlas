//! ProjectDataSource — abstraction over "where does the projects-list /
//! graph / symbol data come from?". Spec A S-004 + future S-006 use this
//! seam to keep handlers testable without spinning up lbug.
//!
//! Two implementations Phase 1:
//!   * `FakeDataSource`     — in-memory, used by integration tests.
//!   * `LbugDataSource`     — real impl wrapping `ga_query` against an
//!                            opened `ga_index::Store`. graph_dump and
//!                            symbol_detail are unimplemented for now
//!                            (need new ga-query helpers — tracked as
//!                            S-004 carve-out destination in
//!                            `.build-checklist`).
//!
//! Errors map to HTTP status codes via `DataError::status_code`.

use serde::Serialize;

// ============== DTOs ==============

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub line_end: Option<u32>,
    pub degree: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub truncated: bool,
    pub total_node_count: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SymbolDetail {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub line_end: Option<u32>,
    pub qualified_name: Option<String>,
    /// Pre-rendered via `ga_query::render::render_signature` — Spec A
    /// AS-035. Stored as a string so the UI panel renders directly
    /// without re-running the render helper.
    pub rendered_signature: String,
    pub layer: Option<String>,

    // ---- Spec E S-003 extension ----
    /// `line_end - line + 1`. None when line_end unknown.
    #[serde(default)]
    pub loc: Option<u32>,
    /// Preview text or None. `has_doc` derived from this on the wire.
    #[serde(default)]
    pub doc_summary: Option<String>,
    #[serde(default)]
    pub has_doc: bool,
    #[serde(default)]
    pub is_async: bool,
    #[serde(default)]
    pub is_abstract: bool,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub is_override: bool,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub is_dead_code: bool,
    #[serde(default)]
    pub is_hub: bool,
    /// `true` when at least one TESTED_BY edge points to this symbol.
    #[serde(default)]
    pub tested: bool,
    #[serde(default)]
    pub caller_count: u32,
    #[serde(default)]
    pub callee_count: u32,
    #[serde(default)]
    pub importer_count: u32,
    /// caller_count + callee_count (RELATIONSHIPS "Impact" line).
    #[serde(default)]
    pub impact_edge_count: u32,
    /// Spec E S-004 — `None` when decoder degrades. Empty vec = arity 0.
    #[serde(default)]
    pub params: Option<Vec<ParamSlotDto>>,
}

/// Wire shape for `SymbolDetail.params` — mirrors
/// `ga_query::render::ParamSlot` but lives in ga-server so the HTTP
/// surface doesn't pull the render crate as a public dep.
#[derive(Debug, Clone, Serialize)]
pub struct ParamSlotDto {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub default_value: String,
}

/// One row in a callers / callees / importers list.
#[derive(Debug, Clone, Serialize)]
pub struct RelationEntry {
    pub id: String,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelationPage {
    pub entries: Vec<RelationEntry>,
    pub total: u64,
    pub has_more: bool,
    pub offset: u64,
    pub limit: u64,
}

// ============== Spec E DTOs ==============

/// Spec E S-001 — a single search hit. `id` is composite
/// `file::name:line` for client-side keying.
#[derive(Debug, Clone, Serialize)]
pub struct SymbolHit {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolSearchResponse {
    pub hits: Vec<SymbolHit>,
    pub truncated: bool,
}

/// Spec E S-002 AS-006 — one row in the layer chip strip.
#[derive(Debug, Clone, Serialize)]
pub struct LayerEntry {
    pub name: String,
    pub symbol_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayersResponse {
    pub layers: Vec<LayerEntry>,
    /// AS-007 — `true` when `architecture()` failed and the UI must
    /// hide the chip strip + flat-group fallback.
    pub degraded: bool,
}

/// Spec E S-002 AS-008/AS-009 — lazy per-layer symbol fetch.
/// `symbols` is for the sidebar tree expand; `symbol_ids` is for the
/// Sigma nodeReducer membership set (canvas highlight/dim).
#[derive(Debug, Clone, Serialize)]
pub struct LayerSymbolsResponse {
    pub symbols: Vec<SymbolHit>,
    pub symbol_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileSummary {
    pub path: String,
    pub language: Option<String>,
    pub line_count: Option<u32>,
    pub symbols: Vec<RelationEntry>,
    pub imports: Vec<String>,         // file paths this file imports
    pub reverse_imports: Vec<String>, // file paths that import this file
}

// ============== Errors ==============

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    /// Slug doesn't resolve to a known project.
    ProjectNotFound,
    /// Symbol id doesn't exist in this project's graph.
    SymbolNotFound,
    /// File path doesn't exist in this project's graph.
    FileNotFound,
    /// Spec E S-002 — layer name not in architecture map.
    LayerNotFound,
    /// Spec E S-001 — pattern failed `is_safe_ident` (maps to 400 bad_pattern).
    BadPattern,
    /// Cache is in `Corrupt` state — Spec A AS-041 / C-cross-8. The
    /// route layer maps this to 503 with a `cache_corrupt` body so the
    /// UI banners "Reindex required".
    CacheCorrupt,
    /// Project hasn't been indexed yet (cache exists but
    /// `index_state: Building` and no committed data yet).
    CacheBuilding,
    /// Anything else.
    Backend(String),
}

impl DataError {
    pub fn error_code(&self) -> &'static str {
        match self {
            DataError::ProjectNotFound => "project_not_found",
            DataError::SymbolNotFound => "symbol_not_found",
            DataError::FileNotFound => "file_not_found",
            DataError::LayerNotFound => "layer_not_found",
            DataError::BadPattern => "bad_pattern",
            DataError::CacheCorrupt => "cache_corrupt",
            DataError::CacheBuilding => "cache_building",
            DataError::Backend(_) => "backend_error",
        }
    }

    pub fn status_code(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            DataError::ProjectNotFound
            | DataError::SymbolNotFound
            | DataError::FileNotFound
            | DataError::LayerNotFound => StatusCode::NOT_FOUND,
            DataError::BadPattern => StatusCode::BAD_REQUEST,
            DataError::CacheCorrupt | DataError::CacheBuilding => StatusCode::SERVICE_UNAVAILABLE,
            DataError::Backend(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            DataError::ProjectNotFound => "project not found".into(),
            DataError::SymbolNotFound => "symbol not found".into(),
            DataError::FileNotFound => "file not found".into(),
            DataError::LayerNotFound => "layer not found".into(),
            DataError::BadPattern => "pattern contains characters outside [A-Za-z0-9_$.]".into(),
            DataError::CacheCorrupt => "reindex required — cache marked corrupt".into(),
            DataError::CacheBuilding => "cache is building; retry after reindex completes".into(),
            DataError::Backend(s) => s.clone(),
        }
    }
}

// ============== Trait ==============

/// Per-project read-only data fetch. All methods take `slug` first so a
/// single instance can serve many projects (the impl decides how to
/// resolve slug → lbug Store handle; FakeDataSource just looks up an
/// in-memory map).
///
/// Send + Sync because handlers run on the axum runtime; Arc<dyn Trait>
/// is the wire format in `AppState`.
pub trait ProjectDataSource: Send + Sync + 'static {
    fn graph_dump(
        &self,
        slug: &str,
        focus: Option<&str>,
        hops: u8,
    ) -> Result<GraphResponse, DataError>;

    fn symbol_detail(&self, slug: &str, symbol_id: &str) -> Result<SymbolDetail, DataError>;

    fn callers(
        &self,
        slug: &str,
        symbol_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError>;

    fn callees(
        &self,
        slug: &str,
        symbol_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError>;

    fn importers(
        &self,
        slug: &str,
        file_path: &str,
        offset: u64,
        limit: u64,
    ) -> Result<RelationPage, DataError>;

    fn file_summary(&self, slug: &str, file_path: &str) -> Result<FileSummary, DataError>;

    /// Spec E S-001 — search symbols by pattern. Pattern is already
    /// `is_safe_ident`-validated by the handler; this layer assumes safe
    /// input. `limit` is the response cap (clamped by handler).
    fn symbols_search(
        &self,
        slug: &str,
        pattern: &str,
        limit: u64,
    ) -> Result<SymbolSearchResponse, DataError>;

    /// Spec E S-002 AS-006/AS-007 — list layers (modules). Implementors
    /// SHOULD return `degraded: true` when the underlying
    /// `architecture()` call fails or yields an empty graph, rather
    /// than propagating an error.
    fn layers(&self, slug: &str) -> Result<LayersResponse, DataError>;

    /// Spec E S-002 AS-008/AS-009 — symbols belonging to one layer
    /// (lazy expand). Returns `LayerNotFound` (mapped to 404) when the
    /// layer name doesn't exist in the architecture map.
    fn layer_symbols(
        &self,
        slug: &str,
        layer_name: &str,
    ) -> Result<LayerSymbolsResponse, DataError>;
}

// ============== Default page size ==============

/// Spec A AS-036 default. Frontend may pass `?limit=` to override.
pub const DEFAULT_PAGE_SIZE: u64 = 50;
/// Hard cap so a malicious caller can't request a million-row page.
pub const MAX_PAGE_SIZE: u64 = 500;

pub fn clamp_limit(req: Option<u64>) -> u64 {
    req.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

/// Spec E S-001 C-1 — search-pattern safety gate. Mirrors
/// `ga_query::common::is_safe_ident` exactly: ASCII alnum + `_` `$` `.`.
/// Empty rejected. We duplicate here so the HTTP layer can return 400
/// before calling into ga_query.
pub fn is_safe_pattern(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
}

/// Spec E S-002 — layer-name path-segment safety gate. Allowlist:
/// ASCII alnum + `_ - . ( )`. Parens are needed because
/// `architecture::dir_basename` returns `(root)` for the repo-root
/// module (empty path); rejecting `()` would make that layer
/// unreachable via URL. Still rejects `/`, `..`, and shell metacharacters.
pub fn is_safe_layer_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '(' | ')'))
        && !s.starts_with('.')
        && s != ".."
}

/// Spec E S-001 AS-002 — search-dropdown cap.
pub const SEARCH_LIMIT_DEFAULT: u64 = 50;
pub const SEARCH_LIMIT_MAX: u64 = 50;

pub fn clamp_search_limit(req: Option<u64>) -> u64 {
    req.unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(1, SEARCH_LIMIT_MAX)
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn pattern_accepts_ident_rejects_paren() {
        assert!(is_safe_pattern("foo_bar"));
        assert!(is_safe_pattern("Foo.Bar"));
        assert!(is_safe_pattern("$x"));
        assert!(!is_safe_pattern(""));
        assert!(!is_safe_pattern("foo()"));
        assert!(!is_safe_pattern("foo/bar"));
    }

    #[test]
    fn layer_name_accepts_paren_root() {
        // architecture::dir_basename returns `(root)` for repo-root module.
        assert!(is_safe_layer_name("(root)"));
        assert!(is_safe_layer_name("ga-query"));
        assert!(is_safe_layer_name("core"));
        assert!(!is_safe_layer_name(""));
        assert!(!is_safe_layer_name(".hidden"));
        assert!(!is_safe_layer_name(".."));
        assert!(!is_safe_layer_name("foo/bar"));
        assert!(!is_safe_layer_name("foo bar"));
    }
}
