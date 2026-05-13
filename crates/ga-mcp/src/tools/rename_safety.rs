//! v1.1-M3 S-004 — `ga_rename_safety` rename impact report (Layer 3 Safety).
//!
//! Tools-C5: read-only contract. Tool returns a `{sites, blocked}` report;
//! the agent decides whether to proceed and invokes text-edit tools
//! separately. Polymorphic confidence resolution per Tools-C11 — `file`
//! hint narrows to a single class on ambiguous targets.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::rename_safety::{rename_safety, RenameSafetyRequest};
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_rename_safety".to_string(),
        description:
            "**Use instead of grep+sed when** the user asks `rename X to Y`, `is this rename \
             safe`, `find all places to update for renaming X`. Returns every site that must \
             change (definition + callers + references + importers) plus blockers (string \
             literals that look like the symbol + external-package collisions). Each site \
             has `confidence` per Tools-C11: 1.0 for definition, 0.90 for CALLS edges, 0.70 \
             for REFERENCES edges, 0.6 for polymorphic-ambiguous dispatch. `grep + sed` \
             misses dispatch-map references and rewrites unrelated identifiers; this tool \
             does not. Optional `file` hint narrows the rename to one class when the name is \
             shared. Read-only — agent invokes edits separately."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["target", "replacement"],
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Identifier to rename (e.g. \"check_password\")."
                },
                "replacement": {
                    "type": "string",
                    "description": "New identifier (must differ from `target`, must match identifier charset)."
                },
                "file": {
                    "type": "string",
                    "description": "Optional Tools-C11 polymorphic narrowing hint — restrict the rename to the class defined in this file."
                },
                "new_arity": {
                    "type": "integer",
                    "description": "Optional v1.3 / S-003 AS-009(b) — proposed new parameter count. When set, the report includes `param_count_changed: true` if the value differs from the target's stored arity (Tools-C2 sentinel `-1` = unknown, never raises the flag)."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<RenameSafetyRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::InvalidParams("ga_rename_safety: arguments must be a JSON object".to_string())
    })?;

    let target = obj
        .get("target")
        .ok_or_else(|| Error::InvalidParams("ga_rename_safety: `target` is required".to_string()))?
        .as_str()
        .ok_or_else(|| {
            Error::InvalidParams("ga_rename_safety: `target` must be a string".to_string())
        })?
        .to_string();

    let replacement = obj
        .get("replacement")
        .ok_or_else(|| {
            Error::InvalidParams("ga_rename_safety: `replacement` is required".to_string())
        })?
        .as_str()
        .ok_or_else(|| {
            Error::InvalidParams("ga_rename_safety: `replacement` must be a string".to_string())
        })?
        .to_string();

    let file_hint = match obj.get("file") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                Error::InvalidParams("ga_rename_safety: `file` must be a string".to_string())
            })?;
            Some(s.to_string())
        }
    };

    // Gap 2 / S-003 AS-009(b) — wire `new_arity` through MCP.
    let new_arity = match obj.get("new_arity") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| {
            Error::InvalidParams("ga_rename_safety: `new_arity` must be an integer".to_string())
        })?),
    };

    Ok(RenameSafetyRequest {
        target,
        replacement,
        file_hint,
        new_arity,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_rename_safety"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;
    let response = rename_safety(ctx.store().as_ref(), &req)?;

    let mut payload = json!({
        "tool": "ga_rename_safety",
        "target": response.target,
        "replacement": response.replacement,
        "sites": response.sites,
        "blocked": response.blocked,
        // Gap 2 / S-003 AS-009(b)
        "existing_arity": response.existing_arity,
        "param_count_changed": response.param_count_changed,
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
