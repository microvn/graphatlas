//! `ga_bridges` — top-N architectural chokepoints (betweenness centrality).
//!
//! Companion to `ga_hubs`: hubs are high-degree, bridges are high-mediation.
//! Mirrors `code-review-graph::get_bridge_nodes_tool` (analysis_tools.py:44).

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::bridges::{bridges, BridgesRequest};
use serde_json::{json, Value};
use std::time::Instant;

const DEFAULT_TOP_N: u32 = 10;
const MAX_TOP_N: u32 = 100;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_bridges".to_string(),
        description: "Top-N architectural chokepoints by betweenness centrality. Bridge nodes \
             sit on the shortest paths between many other symbol pairs — if they break, \
             multiple code regions lose connectivity. Different from `ga_hubs`: a hub \
             has many direct neighbours; a bridge mediates between regions. Uses \
             Brandes' algorithm with deterministic source-vertex sampling (k=500) \
             when the graph exceeds 5000 nodes — `meta.sampled` reports when this \
             approximation kicks in. Excludes external symbols. Hard cap `top_n ≤ 100`. \
             \
             When `symbol` is provided, switches to rank-of-target lookup: returns just \
             that symbol's entry plus `meta.target_rank` (1-based, against the FULL \
             post-Brandes vec — NOT bounded by `top_n`). Unknown `symbol` returns empty \
             `bridges[]` plus top-3 Levenshtein suggestions in `meta.suggestion`. \
             Optional `file` disambiguates same-name symbols across files."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "top_n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TOP_N,
                    "description": "Number of bridges to return. Default 10, max 100. Ignored when `symbol` is set."
                },
                "symbol": {
                    "type": "string",
                    "description": "When set, switches to rank-of-target lookup mode."
                },
                "file": {
                    "type": "string",
                    "description": "Optional file disambiguator. Used only with `symbol`."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<BridgesRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_bridges: arguments must be a JSON object".to_string())
    })?;

    let top_n = match obj.get("top_n") {
        None | Some(Value::Null) => DEFAULT_TOP_N,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                Error::InvalidParams("ga_bridges: `top_n` must be a positive integer".to_string())
            })?;
            if n == 0 {
                return Err(Error::InvalidParams(
                    "ga_bridges: `top_n` must be ≥ 1".to_string(),
                ));
            }
            if n > MAX_TOP_N as u64 {
                return Err(Error::InvalidParams(format!(
                    "ga_bridges: `top_n` must be ≤ {MAX_TOP_N}"
                )));
            }
            n as u32
        }
    };

    let symbol = match obj.get("symbol") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_bridges: `symbol` must be a string".to_string())
            })?;
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    let file = match obj.get("file") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_bridges: `file` must be a string".to_string())
            })?;
            if s.contains('\'') || s.contains('\n') || s.contains('\r') {
                return Err(Error::InvalidParams(
                    "ga_bridges: `file` must not contain quotes or newlines".to_string(),
                ));
            }
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    };

    Ok(BridgesRequest {
        top_n,
        symbol,
        file,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_bridges"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;

    let response = bridges(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_bridges",
        "bridges": response.bridges,
        "meta": {
            "total_nodes": response.meta.total_nodes,
            "total_edges": response.meta.total_edges,
            "sampled": response.meta.sampled,
            "sample_size": response.meta.sample_size,
            "truncated": response.meta.truncated,
            "target_rank": response.meta.target_rank,
            "target_found": response.meta.target_found,
            "suggestion": response.meta.suggestion,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
