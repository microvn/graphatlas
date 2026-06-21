//! Tools S-006 — `ga_impact` MCP tool.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_impact".to_string(),
        description: "**Use instead of grep when** the user asks `impact of changing X`, \
             `blast radius`, `if I change X what breaks`, `what does this PR touch`, or wants \
             impact analysis for a refactor. Flagship one-shot tool: given a symbol, changed \
             files, or a unified git diff, returns impacted files (callers+callees to depth \
             3), affected tests, affected routes, affected configs, a 4-dim runtime risk \
             score (test_gap, blast, depth, exposure — Tools-C18), and break points. Replaces \
             a chain of `grep -r` + manual cross-file reading. At least one of `symbol`, \
             `changed_files`, `diff` is required; precedence symbol > changed_files > diff."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Seed symbol name (optional). Combine with `file` to narrow polymorphic resolution (Tools-C11)."
                },
                "file": {
                    "type": "string",
                    "description": "Narrowing hint for `symbol` resolution — not a strict filter."
                },
                "changed_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Repo-relative file paths whose combined impact to report."
                },
                "diff": {
                    "type": "string",
                    "description": "Unified git diff text; impacted files are extracted from diff headers."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Max BFS depth over callers+callees (default 3)."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<ga_query::ImpactRequest> {
    let obj = args.as_object().ok_or_else(|| {
        Error::Other(anyhow::anyhow!(
            "ga_impact: arguments must be a JSON object"
        ))
    })?;
    // Shape-level check — AS-015 seed-input rule enforced downstream.
    let req: ga_query::ImpactRequest = serde_json::from_value(Value::Object(obj.clone()))
        .map_err(|e| Error::Other(anyhow::anyhow!("ga_impact: invalid arguments: {e}")))?;
    Ok(req)
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_impact"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;
    let response = ga_query::impact(ctx.try_store()?.as_ref(), &req)?;
    // P1.5 (2026-05-22) — Markdown opt-in for impact summaries.
    if crate::markdown::wants_markdown(args) {
        let seed_label = req.symbol.as_deref().unwrap_or_else(|| {
            req.changed_files
                .as_deref()
                .and_then(|fs| fs.first().map(String::as_str))
                .unwrap_or("(diff)")
        });
        let text = crate::markdown::render_impact(
            seed_label,
            &response.impacted_files,
            &response.affected_tests,
            &response.affected_routes,
            &response.affected_configs,
            &response.break_points,
            response.disambiguation.as_ref(),
        );
        let _ = start;
        return Ok(ToolsCallResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
        });
    }

    let mut payload = json!({
        "tool": "ga_impact",
        "impacted_files": response.impacted_files,
        "affected_tests": response.affected_tests,
        "affected_routes": response.affected_routes,
        "affected_configs": response.affected_configs,
        "risk": response.risk,
        "break_points": response.break_points,
        "meta": response.meta,
    });
    // CORE-2 (2026-05-22) — forward disambiguation payload when seed is
    // ambiguous (multi-def + no file hint).
    if let Some(dis) = response.disambiguation.as_ref() {
        payload["disambiguation"] = json!(dis);
    }
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
