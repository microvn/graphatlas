//! Tools S-004 — `ga_symbols` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_symbols".to_string(),
        description: "**Use instead of grep when** the user asks `where is X defined`, \
             `find symbol X`, `lookup X`, or names a symbol they want to inspect. Searches \
             the indexed symbol table — grep returns every textual occurrence (uses, \
             comments, strings); this returns only definitions, ranked by relevance. \
             `match: \"exact\"` ranks same-name defs by caller-count (popular defs win); \
             `match: \"fuzzy\"` ranks by Levenshtein distance for typos / partial recall. \
             Capped at 10 results — `meta.truncated` + `meta.total_available` expose what \
             was elided. Run this FIRST when a symbol name is ambiguous, then chain to \
             ga_callers / ga_callees / ga_impact with the resolved name."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Symbol name or fuzzy query. Allowed chars: [A-Za-z0-9_$.] (Tools-C9-d)."
                },
                "match": {
                    "type": "string",
                    "enum": ["exact", "fuzzy"],
                    "description": "Matching mode. Defaults to `exact` when omitted."
                }
            },
            "required": ["pattern"]
        }),
    }
}

fn validate_args(args: &Value) -> Result<(&str, ga_query::SymbolsMatch)> {
    let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
        return Err(Error::Other(anyhow::anyhow!(
            "ga_symbols: `pattern` is a required string argument"
        )));
    };
    let mode = match args.get("match").and_then(|v| v.as_str()) {
        Some("fuzzy") => ga_query::SymbolsMatch::Fuzzy,
        Some("exact") | None => ga_query::SymbolsMatch::Exact,
        Some(other) => {
            return Err(Error::Other(anyhow::anyhow!(
                "ga_symbols: `match` must be \"exact\" or \"fuzzy\" (got {other:?})"
            )))
        }
    };
    Ok((pattern, mode))
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_symbols"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let (pattern, mode) = validate_args(args)?;
    let response = ga_query::symbols(ctx.store().as_ref(), pattern, mode)?;
    let mut payload = json!({
        "tool": "ga_symbols",
        "pattern": pattern,
        "symbols": response.symbols,
        "meta": {
            "truncated": response.meta.truncated,
            "total_available": response.meta.total_available,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
