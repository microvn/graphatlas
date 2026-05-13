//! Shared argument validators + error builders used by per-tool dispatchers.

use crate::context::McpContext;
use ga_core::{Error, Result};
use serde_json::{json, Value};
use std::time::Instant;

/// Tools-C1 — merge `query_time_ms`, `cache_hit`, `graph_version` into the
/// `meta` object of `payload` in place. Pass the `Instant` captured at the
/// entry of the per-tool `call` so timing reflects the full dispatch.
///
/// `cache_hit` is stubbed `true` for v1: the MCP server always queries a
/// pre-built index (build_index ran during `Store::open_with_root`). When
/// lazy / partial caching lands post-v1, flip this to the real flag.
pub(super) fn inject_common_meta(payload: &mut Value, ctx: &McpContext, start: Instant) {
    if let Some(meta) = payload.get_mut("meta").and_then(|v| v.as_object_mut()) {
        meta.insert(
            "query_time_ms".into(),
            json!(start.elapsed().as_millis() as u64),
        );
        meta.insert("cache_hit".into(), json!(true));
        meta.insert(
            "graph_version".into(),
            json!(ctx.store().metadata().schema_version),
        );
    }
}

pub(super) fn validate_symbol_file_args<'a>(
    args: &'a Value,
    tool: &str,
) -> Result<(&'a str, Option<&'a str>)> {
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        return Err(Error::Other(anyhow::anyhow!(
            "{tool}: `symbol` is a required string argument"
        )));
    };
    let file = args.get("file").and_then(|v| v.as_str());
    Ok((symbol, file))
}

pub(super) fn validate_file_arg<'a>(args: &'a Value, tool: &str) -> Result<&'a str> {
    let Some(file) = args.get("file").and_then(|v| v.as_str()) else {
        return Err(Error::Other(anyhow::anyhow!(
            "{tool}: `file` is a required string argument"
        )));
    };
    Ok(file)
}

pub(super) fn validate_path_arg<'a>(args: &'a Value, tool: &str) -> Result<&'a str> {
    let Some(path) = args.get("path").and_then(|v| v.as_str()) else {
        return Err(Error::Other(anyhow::anyhow!(
            "{tool}: `path` is a required string argument"
        )));
    };
    Ok(path)
}

pub(super) fn store_ctx_required_error(tool: &str) -> Error {
    Error::Other(anyhow::anyhow!(
        "{tool} requires a Store context; caller must use dispatch_tool_call_with_ctx"
    ))
}
