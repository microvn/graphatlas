//! v1.1-M3 S-003 — `ga_dead_code` entry-point-aware dead-code detector
//! (Layer 3 Safety).
//!
//! Tools-C4: entry-point detection covers framework routes (gin/django/
//! rails/axum/nest), CLI commands (`[project.scripts]`, clap derive,
//! Cobra), `main` functions, and library public API (`__all__`, `pub use`,
//! `export {}`). Symbols matched by any of these channels are filtered
//! before the response is built so the LLM sees actual cleanup candidates.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::dead_code::{dead_code, DeadCodeRequest};
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_dead_code".to_string(),
        description: "**Use instead of grep when** the user asks `find dead code`, `unused \
             functions`, `code to delete`, or `what's safe to remove`. Lists symbols with \
             zero in-degree (no callers + no value-references) AFTER filtering known entry \
             points: framework routes (gin/django/rails/axum/nest), CLI commands \
             (`[project.scripts]`), `main` / `__main__`, library public API (Python \
             `__all__`), and test functions. Plain grep cannot distinguish a function with \
             no callers from a route handler mounted via decorator — this tool does. Each \
             entry carries `confidence ≥ 0.80` per AS-008. Optional `scope` argument \
             restricts analysis to a path prefix (e.g. `src/utils/`)."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "description": "Optional path prefix to scope analysis (e.g. `src/utils/`). Empty string ≡ full repo."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<DeadCodeRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_dead_code: arguments must be a JSON object".to_string())
    })?;

    let scope = match obj.get("scope") {
        None => None,
        Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_dead_code: `scope` must be a string".to_string())
            })?;
            Some(s.to_string())
        }
    };

    Ok(DeadCodeRequest { scope })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_dead_code"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;

    let response = dead_code(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_dead_code",
        "dead": response.dead,
        "meta": {
            "total_zero_caller": response.meta.total_zero_caller,
            "entry_point_filtered": response.meta.entry_point_filtered,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
