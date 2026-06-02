//! Shared argument validators + error builders used by per-tool dispatchers.

use crate::context::McpContext;
use ga_core::{Error, Result};
use serde_json::{json, Value};
use std::time::Instant;

/// Tools-C1 — merge `query_time_ms`, `cache_hit`, `graph_version` into the
/// `meta` object of `payload` in place. Pass the `Instant` captured at the
/// entry of the per-tool `call` so timing reflects the full dispatch.
///
/// `cache_hit` reports whether this *response* was served from a result cache.
/// There is no response cache today — every tool call re-queries the live index
/// — so the honest value is `false`. (Earlier this was hardcoded `true`, which
/// misreported a hit on every call.) When result caching lands, wire the real
/// per-call flag here.
pub(super) fn inject_common_meta(payload: &mut Value, ctx: &McpContext, start: Instant) {
    if let Some(meta) = payload.get_mut("meta").and_then(|v| v.as_object_mut()) {
        meta.insert(
            "query_time_ms".into(),
            json!(start.elapsed().as_millis() as u64),
        );
        meta.insert("cache_hit".into(), json!(false));
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

/// P1.3 (2026-05-22) — caller opt-in to keep polymorphic conf 0.6 entries.
/// Default is `false` → wrapper drops `confidence < 1.0` entries to avoid
/// flooding the LLM with same-name-from-other-file noise that the `file:`
/// hint was meant to disambiguate. Restore the legacy fan-out with
/// `include_uncertain: true`.
pub(super) fn wants_include_uncertain(args: &Value) -> bool {
    args.get("include_uncertain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
