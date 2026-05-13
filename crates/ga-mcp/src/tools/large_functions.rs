//! `ga_large_functions` — symbols whose line span exceeds a threshold.
//!
//! Mirrors `code-review-graph::find_large_functions_tool`. Decomposition
//! target finder for code review + refactor planning.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::large_functions::{large_functions, LargeFunctionsRequest};
use serde_json::{json, Value};
use std::time::Instant;

const DEFAULT_MIN_LINES: u32 = 50;
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_large_functions".to_string(),
        description: "List symbols whose line span (`line_end - line + 1`) meets or exceeds \
             `min_lines`. Use to surface decomposition targets, oversized handlers, \
             or untested complexity hotspots. Supports `kind` filter (e.g. `function`, \
             `method`, `class`) and `file_pattern` substring filter (e.g. `src/utils/`). \
             Returns name, file, kind, line, line_end, line_count — sorted by line_count \
             DESC. Hard cap `limit ≤ 200`."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "min_lines": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Inclusive minimum line span. Default 50."
                },
                "kind": {
                    "type": "string",
                    "description": "Optional kind filter (function/method/class/...)."
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Optional file-path substring filter (e.g. `src/utils/`)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": "Result cap. Default 50, max 200."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<LargeFunctionsRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_large_functions: arguments must be a JSON object".to_string())
    })?;

    let min_lines = match obj.get("min_lines") {
        None | Some(Value::Null) => DEFAULT_MIN_LINES,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                Error::InvalidParams(
                    "ga_large_functions: `min_lines` must be a positive integer".to_string(),
                )
            })?;
            if n == 0 {
                return Err(Error::InvalidParams(
                    "ga_large_functions: `min_lines` must be ≥ 1".to_string(),
                ));
            }
            n as u32
        }
    };

    let kind = match obj.get("kind") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_large_functions: `kind` must be a string".to_string())
            })?;
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    };

    let file_pattern = match obj.get("file_pattern") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams(
                    "ga_large_functions: `file_pattern` must be a string".to_string(),
                )
            })?;
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    };

    let limit = match obj.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                Error::InvalidParams(
                    "ga_large_functions: `limit` must be a positive integer".to_string(),
                )
            })?;
            if n == 0 {
                return Err(Error::InvalidParams(
                    "ga_large_functions: `limit` must be ≥ 1".to_string(),
                ));
            }
            if n > MAX_LIMIT as u64 {
                return Err(Error::InvalidParams(format!(
                    "ga_large_functions: `limit` must be ≤ {MAX_LIMIT}"
                )));
            }
            n as u32
        }
    };

    Ok(LargeFunctionsRequest {
        min_lines,
        kind,
        file_pattern,
        limit,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_large_functions"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;

    let response = large_functions(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_large_functions",
        "functions": response.functions,
        "meta": {
            "total_matches": response.meta.total_matches,
            "truncated": response.meta.truncated,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
