//! Tools S-005 — `ga_file_summary` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error, validate_path_arg};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::Result;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_file_summary".to_string(),
        description: "**Use instead of Read when** the user asks `what does file F do`, \
             `give me an overview of F`, `outline F`, or wants the shape of a file before \
             deciding to read it in full. Returns defined symbols (ordered by line, with \
             kind), imported files (repo-local), and exported names — typically a few KB \
             vs reading the entire file. Use this BEFORE `Read` to scope the source you \
             actually need to load."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repo-relative file path as stored in the graph."
                }
            },
            "required": ["path"]
        }),
    }
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_path_arg(args, "ga_file_summary")?;
    Err(store_ctx_required_error("ga_file_summary"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let path = validate_path_arg(args, "ga_file_summary")?;
    let summary = ga_query::file_summary(ctx.store().as_ref(), path)?;
    let mut payload = json!({
        "tool": "ga_file_summary",
        "path": summary.path,
        "symbols": summary.symbols,
        "imports": summary.imports,
        "exports": summary.exports,
        "meta": {},
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
