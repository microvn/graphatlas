//! v1.1-M3 S-005 — `ga_architecture` module map (Layer 4 Meta).
//!
//! Tools-C6: response `meta.convention_used` names which discovery
//! convention was applied (`python-init-py` / `cargo` / `node-package`,
//! comma-joined for polyglot repos, `none` for flat repos).
//!
//! Read-only contract per Tools-C5.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::architecture::{architecture, ArchitectureRequest};
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_architecture".to_string(),
        description:
            "**Use instead of `ls` + Read when** the user asks `architecture of this repo`, \
             `orient me`, `what are the main modules`, `getting started in this codebase`, \
             or wants a top-down map before diving in. Module map of the indexed repo: \
             `modules` (one per Python package / Cargo crate / npm workspace member, with \
             `name`, `files`, `symbol_count`, `public_api`) + `edges` (inter-module CALLS / \
             IMPORTS / EXTENDS aggregated by weight). Use BEFORE reading any file in an \
             unfamiliar repo. Optional `max_modules` caps the response to top-N by \
             symbol_count; `meta.truncated` + `meta.total_modules` surface what was hidden. \
             `meta.convention_used` names which discovery convention applied per Tools-C6."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "max_modules": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional cap on returned modules (top-N by symbol_count). Omit for no cap. Must be ≥ 1."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<ArchitectureRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_architecture: arguments must be a JSON object".to_string())
    })?;

    let max_modules = match obj.get("max_modules") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_i64().ok_or_else(|| {
                Error::InvalidParams(
                    "ga_architecture: `max_modules` must be a positive integer".to_string(),
                )
            })?;
            if n < 1 {
                return Err(Error::InvalidParams(
                    "ga_architecture: `max_modules` must be ≥ 1 (or omitted for no cap)"
                        .to_string(),
                ));
            }
            Some(n as u32)
        }
    };

    Ok(ArchitectureRequest { max_modules })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_architecture"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;
    let response = architecture(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_architecture",
        "modules": response.modules,
        "edges": response.edges,
        "meta": {
            "truncated": response.meta.truncated,
            "total_modules": response.meta.total_modules,
            "convention_used": response.meta.convention_used,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
