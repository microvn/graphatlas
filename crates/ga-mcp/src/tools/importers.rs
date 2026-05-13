//! Tools S-003 — `ga_importers` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error, validate_file_arg};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::Result;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_importers".to_string(),
        description: "List files that import the given file. Use to estimate change blast radius \
             at file granularity. Response entries flag `re_export: true` and set `via` for \
             transitive importers surfaced through `export * from '…'` / `export { X } from '…'` \
             chains up to 3 hops deep."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Target file path (as stored in the graph, repo-relative)."
                }
            },
            "required": ["file"]
        }),
    }
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_file_arg(args, "ga_importers")?;
    Err(store_ctx_required_error("ga_importers"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let file = validate_file_arg(args, "ga_importers")?;
    let response = ga_query::importers(ctx.store().as_ref(), file)?;
    let mut payload = json!({
        "tool": "ga_importers",
        "file": file,
        "importers": response.importers,
        "meta": {},
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
