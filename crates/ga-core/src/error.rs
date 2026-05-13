use thiserror::Error;

/// Typed error boundary between library crates.
/// Per Foundation-C5 + AS-023: MCP server maps variants to JSON-RPC codes −32000..−32099.
#[derive(Debug, Error)]
pub enum Error {
    #[error("index not ready (status={status}, progress={progress})")]
    IndexNotReady { status: String, progress: f32 },

    #[error("parse error in {file} ({lang}): {err}")]
    ParseError {
        file: String,
        lang: String,
        err: String,
    },

    #[error("config corrupt at {path}: {reason}")]
    ConfigCorrupt { path: String, reason: String },

    #[error("schema version mismatch: cache={cache}, binary={binary}")]
    SchemaVersionMismatch { cache: u32, binary: u32 },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Database(String),

    /// JSON-RPC -32602. Used by tool input validators that reject a request
    /// whose fields violate the tool's contract (e.g. `ga_impact` with no
    /// `symbol` / `changed_files` / `diff` set).
    #[error("invalid params: {0}")]
    InvalidParams(String),

    /// JSON-RPC -32602 (sub-variant) — symbol/seed lookup found nothing in the
    /// index. Emits structured `data: {suggestions: [...]}` per
    /// graphatlas-v1.1-tools.md AS-016 and AS-004 (ga_risk symbol-not-found).
    /// `suggestions` is the top-N Levenshtein-ranked nearest-symbol-names
    /// (capped at 3 per spec).
    #[error("symbol not found{}", if .suggestions.is_empty() { String::new() } else { format!(" (did you mean: {})", .suggestions.join(", ")) })]
    SymbolNotFound { suggestions: Vec<String> },

    /// v1.5 PR5 Staleness Phase C — JSON-RPC -32010. The MCP staleness gate
    /// detected drift between `metadata.indexed_root_hash` and the live
    /// Merkle root of the repo. Default response: refuse the tool dispatch
    /// and emit this error so the agent calls `ga_reindex` to recover.
    /// Bypass: pass `allow_stale: true` in the tool args to receive the
    /// stale data anyway (with `meta.stale: true` in the response).
    ///
    /// `indexed_root` / `current_root` are lowercase hex strings (64 chars
    /// each) — same shape as `Metadata.indexed_root_hash`. `drift_detected_at`
    /// is unix seconds at the moment the gate fired.
    #[error("Index out of date. Call ga_reindex first.")]
    StaleIndex {
        indexed_root: String,
        current_root: String,
        drift_detected_at: u64,
        /// v1.5 S-004 AS-014 — Tier 2 BLAKE3 dirty-paths gate populates
        /// this list with the modified file paths it found (relative
        /// to repo root, forward-slash normalized). Empty when the
        /// error fired from Tier 1 Merkle (which doesn't know per-file
        /// detail). The `to_jsonrpc_error` mapper surfaces non-empty
        /// values under `data.dirty_paths` so agents can decide
        /// whether to call `ga_reindex` or re-prompt the user.
        dirty_paths: Vec<String>,
    },

    /// v1.5 PR6.1b — JSON-RPC -32012. `ga_reindex` consumed the old Store,
    /// nuked the cache, but the fresh `build_index` (or the constructor's
    /// reopen step) failed. Disk cache is empty; the McpContext's inner
    /// store cell is left in the `None` sentinel state so the next tool
    /// call surfaces a clear error rather than silently serving stale data.
    #[error("ga_reindex build failed: {reason}")]
    ReindexBuildFailed { reason: String },

    /// v1.5 PR6.1b — JSON-RPC -32013. `ga_reindex` could not take exclusive
    /// ownership of the in-process Store because other tool calls still
    /// hold `Arc<Store>` clones (refcount > 1). Caller should retry after
    /// outstanding tool calls drain.
    #[error("ga_reindex blocked: in-flight tool calls still hold the store (retry shortly)")]
    StoreBusy,

    /// v1.5 PR6.1d — JSON-RPC -32014. A peer process is already running
    /// `ga_reindex` against the same cache_root, or the in-process
    /// post-success cooldown is still active. Caller should wait + retry,
    /// or accept that the index is already being refreshed.
    #[error("ga_reindex already in progress (peer process or 200ms cooldown active)")]
    AlreadyReindexing { hint: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// JSON-RPC error code per AS-023 mapping.
    pub fn jsonrpc_code(&self) -> i32 {
        match self {
            Self::IndexNotReady { .. } => -32000,
            Self::ParseError { .. } => -32001,
            Self::ConfigCorrupt { .. } => -32002,
            Self::SchemaVersionMismatch { .. } => -32003,
            Self::Io(_) => -32004,
            Self::Database(_) => -32005,
            Self::InvalidParams(_) => -32602,
            Self::SymbolNotFound { .. } => -32602,
            // v1.5 PR5 — Staleness gate. -32010 is the first slot in the
            // GA-reserved range -32010..-32099 (per challenge clarification
            // on error code allocation).
            Self::StaleIndex { .. } => -32010,
            Self::ReindexBuildFailed { .. } => -32012,
            Self::StoreBusy => -32013,
            Self::AlreadyReindexing { .. } => -32014,
            Self::Other(_) => -32099,
        }
    }
}
