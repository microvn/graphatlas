//! Tools S-001 — `ga_callers` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error, validate_symbol_file_args};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::Result;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_callers".to_string(),
        description: "List functions that call OR reference the given symbol. Use before \
             refactoring to assess blast radius. Each entry has `kind: \"call\"` (direct \
             invocation) or `kind: \"reference\"` (symbol held by value — dispatch map, \
             callback array, shorthand property). Caller fields include file, symbol name, \
             definition line, and call-site / reference-site line."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Target symbol name (function / method / class)."
                },
                "file": {
                    "type": "string",
                    "description": "Optional: restrict search to callers defined inside this file path."
                }
            },
            "required": ["symbol"]
        }),
    }
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_symbol_file_args(args, "ga_callers")?;
    Err(store_ctx_required_error("ga_callers"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let (symbol, file) = validate_symbol_file_args(args, "ga_callers")?;
    let response = ga_query::callers(ctx.store().as_ref(), symbol, file)?;
    let mut payload = json!({
        "tool": "ga_callers",
        "symbol": symbol,
        "file": file,
        "callers": response.callers,
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
