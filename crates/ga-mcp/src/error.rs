//! AS-023 — map `ga_core::Error` → JSON-RPC error envelope.
//!
//! Error codes per the spec table:
//!   -32000 IndexNotReady         (data: status, progress, eta_sec)
//!   -32001 ParseError            (data: file, lang, err)
//!   -32002 ConfigCorrupt         (data: path, reason)
//!   -32003 SchemaVersionMismatch (data: cache, binary)
//!   -32004 IoError
//!   -32005 DatabaseError
//!   -32602 InvalidParams        (data: none — message carries the detail)
//!   -32099 Other

use ga_core::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Convert a `ga_core::Error` into the JSON-RPC error envelope described by
/// AS-023. User-facing `message` always hints at `graphatlas doctor` for
/// recoverable conditions.
pub fn to_jsonrpc_error(err: &Error) -> JsonRpcError {
    let code = err.jsonrpc_code();
    match err {
        Error::IndexNotReady { status, progress } => JsonRpcError {
            code,
            message: "Index not ready — run `graphatlas doctor` for status.".to_string(),
            data: Some(serde_json::json!({
                "status": status,
                "progress": progress,
                // eta_sec: best-effort; null on first-run / unknown.
                "eta_sec": Value::Null,
            })),
        },
        Error::ParseError { file, lang, err } => JsonRpcError {
            code,
            message: format!(
                "Parse error in {file} ({lang}): {err}. \
                 Run `graphatlas doctor` if this persists."
            ),
            data: Some(serde_json::json!({
                "file": file,
                "lang": lang,
                "err": err,
            })),
        },
        Error::ConfigCorrupt { path, reason } => JsonRpcError {
            code,
            message: format!(
                "Config corrupt at {path}: {reason}. \
                 Run `graphatlas doctor` to diagnose."
            ),
            data: Some(serde_json::json!({
                "path": path,
                "reason": reason,
            })),
        },
        Error::SchemaVersionMismatch { cache, binary } => JsonRpcError {
            code,
            message: format!(
                "Cache schema v{cache} does not match binary v{binary}. \
                 Rebuilding automatically; run `graphatlas doctor` if \
                 this persists."
            ),
            data: Some(serde_json::json!({
                "cache": cache,
                "binary": binary,
            })),
        },
        Error::Io(e) => JsonRpcError {
            code,
            message: format!("I/O error: {e}. Run `graphatlas doctor`."),
            data: None,
        },
        Error::Database(msg) => JsonRpcError {
            code,
            message: format!("Database error: {msg}. Run `graphatlas doctor`."),
            data: None,
        },
        Error::InvalidParams(msg) => JsonRpcError {
            code,
            message: format!("Invalid params: {msg}"),
            data: None,
        },
        // AS-016 / AS-004 — structured `data.suggestions` per spec literal:
        // `{code: -32602, message: "Symbol not found", data: {suggestions: [...]}}`.
        // Distinguished from the generic InvalidParams variant so the LLM
        // agent can pattern-match `code == -32602 && message starts with
        // "Symbol not found"` to extract the suggestion array directly.
        Error::SymbolNotFound { suggestions } => JsonRpcError {
            code,
            message: "Symbol not found".to_string(),
            data: Some(serde_json::json!({
                "suggestions": suggestions,
            })),
        },
        // v1.5 PR5 — staleness gate. -32010 with structured data per
        // staleness sub-spec AS-008. Message is the user-visible "call
        // ga_reindex first" hint; data carries the hashes + drift time so
        // agents can decide whether to retry-with-allow_stale or call reindex.
        Error::StaleIndex {
            indexed_root,
            current_root,
            drift_detected_at,
            dirty_paths,
        } => {
            let mut data = serde_json::json!({
                "indexed_root": indexed_root,
                "current_root": current_root,
                "drift_detected_at": drift_detected_at,
            });
            // S-004 AS-014 — surface Tier 2 per-file dirty list when
            // populated. Tier 1 errors leave it empty + the field is
            // omitted from the envelope.
            if !dirty_paths.is_empty() {
                data.as_object_mut().unwrap().insert(
                    "dirty_paths".to_string(),
                    serde_json::Value::Array(
                        dirty_paths
                            .iter()
                            .map(|p| serde_json::Value::String(p.clone()))
                            .collect(),
                    ),
                );
            }
            JsonRpcError {
                code,
                message: "Index out of date. Call ga_reindex first.".to_string(),
                data: Some(data),
            }
        }
        // v1.5 PR6.1b — ga_reindex rebuild failure. -32012 with a `reason`
        // string so agents can decide whether to retry, abort, or escalate.
        Error::ReindexBuildFailed { reason } => JsonRpcError {
            code,
            message: format!("ga_reindex build failed: {reason}"),
            data: Some(serde_json::json!({ "reason": reason })),
        },
        // v1.5 PR6.1b — ga_reindex blocked by in-flight readers (refcount > 1).
        // -32013. Caller should retry after outstanding tool calls drain.
        // v1.5 PR6.1d — already-reindexing (peer or cooldown). -32014 with
        // a `hint` string so agents can decide retry policy.
        Error::AlreadyReindexing { hint } => JsonRpcError {
            code,
            message: format!("ga_reindex already in progress: {hint}"),
            data: Some(serde_json::json!({ "hint": hint })),
        },
        Error::StoreBusy => JsonRpcError {
            code,
            message: "ga_reindex blocked: in-flight tool calls still hold the store \
                      (retry shortly)."
                .to_string(),
            data: None,
        },
        Error::Other(e) => JsonRpcError {
            code,
            message: format!("Internal error: {e}"),
            data: None,
        },
    }
}
