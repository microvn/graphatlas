//! `ga_hubs` — top-N most-connected symbols (architectural hotspots).
//!
//! Mirrors `code-review-graph::get_hub_nodes_tool` (analysis_tools.py:17).
//! Used to surface architectural backbone: changes to a hub have outsized
//! blast radius, so reviewers + LLMs benefit from knowing them up front.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::hubs::{hubs, HubsRequest};
use serde_json::{json, Value};
use std::time::Instant;

const DEFAULT_TOP_N: u32 = 10;
const MAX_TOP_N: u32 = 100;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_hubs".to_string(),
        description: "Top-N most-connected symbols in the indexed repo (architectural hotspots). \
             Score = sum of incoming + outgoing CALLS and REFERENCES edges. Use BEFORE \
             touching unfamiliar code to spot symbols whose changes have outsized blast \
             radius. Returns `name`, `file`, `kind`, `line`, `in_degree`, `out_degree`, \
             `total_degree` per entry, sorted by `total_degree` DESC. Excludes external \
             symbols. Hard cap `top_n ≤ 100`. \
             \
             When `symbol` is provided, switches to rank-of-target lookup: returns just \
             that symbol's entry plus `meta.target_rank` (1-based position in the FULL \
             ranking, NOT bounded by `top_n`). If `symbol` is unknown, returns empty \
             `hubs[]` plus top-3 Levenshtein suggestions in `meta.suggestion`. Optional \
             `file` disambiguates same-name symbols across files."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "top_n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TOP_N,
                    "description": "Number of hubs to return. Default 10, max 100. Ignored when `symbol` is set."
                },
                "symbol": {
                    "type": "string",
                    "description": "When set, switches to rank-of-target lookup mode. Returns the matching entry + `meta.target_rank`. Mutually exclusive with the top-N use case."
                },
                "file": {
                    "type": "string",
                    "description": "Optional file disambiguator for same-name symbols. Used only with `symbol`."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<HubsRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_hubs: arguments must be a JSON object".to_string())
    })?;

    let top_n = match obj.get("top_n") {
        None | Some(Value::Null) => DEFAULT_TOP_N,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                Error::InvalidParams("ga_hubs: `top_n` must be a positive integer".to_string())
            })?;
            if n == 0 {
                return Err(Error::InvalidParams(
                    "ga_hubs: `top_n` must be ≥ 1".to_string(),
                ));
            }
            if n > MAX_TOP_N as u64 {
                return Err(Error::InvalidParams(format!(
                    "ga_hubs: `top_n` must be ≤ {MAX_TOP_N}"
                )));
            }
            n as u32
        }
    };

    let symbol = match obj.get("symbol") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_hubs: `symbol` must be a string".to_string())
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
                Error::InvalidParams("ga_hubs: `file` must be a string".to_string())
            })?;
            if s.contains('\'') || s.contains('\n') || s.contains('\r') {
                return Err(Error::InvalidParams(
                    "ga_hubs: `file` must not contain quotes or newlines".to_string(),
                ));
            }
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    };

    Ok(HubsRequest {
        top_n,
        symbol,
        file,
        edge_types: ga_query::hubs::HubsEdgeTypes::Default,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_hubs"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;

    let response = hubs(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_hubs",
        "hubs": response.hubs,
        "meta": {
            "total_symbols_with_edges": response.meta.total_symbols_with_edges,
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
