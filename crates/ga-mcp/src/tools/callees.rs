//! Tools S-002 — `ga_callees` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error, validate_symbol_file_args};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::Result;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_callees".to_string(),
        description: "**Use instead of grep when** the user asks `what does X call`, \
             `dependencies of X`, `what's used by X`, or wants to understand a symbol's \
             outgoing dependencies before extracting / moving / splitting it. Reads typed \
             CALL + REFERENCES edges — grep on a function body returns identifier matches \
             but cannot tell call sites from string literals or comments. Each entry has \
             `kind: \"call\"` or `kind: \"reference\"`. External (stdlib / third-party) \
             callees are flagged `external: true`; `symbol_kind` exposes the type \
             (`function`, `method`, `class`, ...)."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Source symbol name whose outgoing calls you want."
                },
                "file": {
                    "type": "string",
                    "description": "Optional narrowing hint — when the name is defined in multiple files, \
                        callers in this file get confidence 1.0, others 0.6 (Tools-C11)."
                }
            },
            "required": ["symbol"]
        }),
    }
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_symbol_file_args(args, "ga_callees")?;
    Err(store_ctx_required_error("ga_callees"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let (symbol, file) = validate_symbol_file_args(args, "ga_callees")?;
    let response = ga_query::callees(ctx.store().as_ref(), symbol, file)?;
    let mut payload = json!({
        "tool": "ga_callees",
        "symbol": symbol,
        "file": file,
        "callees": response.callees,
        "meta": {
            "symbol_found": response.meta.symbol_found,
            "suggestion": response.meta.suggestion,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
