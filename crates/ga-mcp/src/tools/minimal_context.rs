//! v1.1-M2 S-002 — `ga_minimal_context` token-budgeted retriever (Layer 2).
//!
//! Tools-C3: token count is an approximation (char-count/4) — the
//! `inputSchema.description` documents this so LLM agents know the
//! ±10% error envelope. `tiktoken-rs` integration is opt-in future
//! work; the API stays stable so a swap is internal.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::minimal_context::{minimal_context, MinimalContextRequest};
use serde_json::{json, Value};
use std::time::Instant;

/// Default budget if caller omits `budget`. Matches the AS-005 example
/// (2000 tokens) — typical Claude/GPT budget slice for an LLM-agent
/// tool-call.
const DEFAULT_BUDGET: u32 = 2000;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_minimal_context".to_string(),
        description: "**Use instead of Read when** the user asks `minimal context for X`, \
             `what do I need to understand X`, `give me just enough to read X`, or you need \
             to fit a symbol + its surrounding code into a token budget. Given `symbol` or \
             `file` + `budget`, returns the smallest slice (seed body + top callers' \
             signatures + top callees' signatures) that fits. Reading the whole file is \
             often 10× the tokens you need; this is the budgeted alternative. Priority \
             when budget binds: seed body > callers signatures > callees signatures > \
             imported types."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Seed symbol name. Mutually exclusive with `file`."
                },
                "file": {
                    "type": "string",
                    "description": "Repo-relative file path for file-level context. Mutually exclusive with `symbol`."
                },
                "budget": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Token budget. Approximation note (Tools-C3): tokens are estimated via char-count / 4 — within ±10% of GPT-style tokenizers on natural-language and source code (no tiktoken dep). When omitted, defaults to 2000."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<MinimalContextRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::Other(anyhow::anyhow!(
            "ga_minimal_context: arguments must be a JSON object"
        ))
    })?;

    let symbol = obj
        .get("symbol")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let file = obj
        .get("file")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let budget = match obj.get("budget") {
        None => DEFAULT_BUDGET,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| {
                Error::InvalidParams(
                    "ga_minimal_context: `budget` must be a non-negative integer".to_string(),
                )
            })?
            .min(u32::MAX as u64) as u32,
    };

    if symbol.is_none() && file.is_none() {
        return Err(Error::InvalidParams(
            "ga_minimal_context: at least one of `symbol` or `file` is required".to_string(),
        ));
    }
    if symbol.is_some() && file.is_some() {
        return Err(Error::InvalidParams(
            "ga_minimal_context: `symbol` and `file` are mutually exclusive".to_string(),
        ));
    }

    Ok(MinimalContextRequest {
        symbol,
        file,
        budget,
        seed_file_hint: None,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_minimal_context"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;
    let response = minimal_context(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_minimal_context",
        "symbols": response.symbols,
        "token_estimate": response.token_estimate,
        "budget_used": response.budget_used,
        "meta": {
            "truncated": response.meta.truncated,
            "warning": response.meta.warning,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
