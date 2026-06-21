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
                },
                "format": {
                    "type": "string",
                    "enum": ["json", "markdown"],
                    "description": "Output format. `markdown` is ~50% cheaper in tokens."
                },
                "include_uncertain": {
                    "type": "boolean",
                    "description": "Opt-in to include polymorphic conf-0.6 entries. Default: false."
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
    let mut response = ga_query::callees(ctx.try_store()?.as_ref(), symbol, file)?;

    // P1.3 (2026-05-22) — drop conf < 1.0 entries unless caller opts in.
    let include_uncertain = super::common::wants_include_uncertain(args);
    let hidden_uncertain_count = if include_uncertain {
        0
    } else {
        let before = response.callees.len();
        response.callees.retain(|c| c.confidence >= 1.0);
        (before - response.callees.len()) as u64
    };

    // P3.1 (2026-05-22) — compact default.
    let use_compact = crate::compact::wants_compact(args);

    // P1.5 (2026-05-22) — Markdown opt-in.
    if crate::markdown::wants_markdown(args) {
        let text = if use_compact && response.disambiguation.is_none() {
            let compact = crate::compact::compact_callees(response.callees);
            let mut s = format!(
                "## Callees of {symbol} ({} callees, {} sites)\n",
                compact.len(),
                compact.iter().map(|c| c.call_site_count).sum::<u32>()
            );
            for c in &compact {
                s.push_str(&crate::compact::render_compact_callee_md(c));
                s.push('\n');
            }
            s
        } else {
            crate::markdown::render_callees(
                symbol,
                &response.callees,
                response.disambiguation.as_ref(),
            )
        };
        let _ = start;
        return Ok(ToolsCallResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
        });
    }

    // JSON path — compact callees when no ambiguity AND default verbosity.
    let callees_payload = if use_compact && response.disambiguation.is_none() {
        json!(crate::compact::compact_callees(response.callees))
    } else {
        json!(response.callees)
    };

    let mut payload = json!({
        "tool": "ga_callees",
        "symbol": symbol,
        "file": file,
        "callees": callees_payload,
        "meta": {
            "symbol_found": response.meta.symbol_found,
            "suggestion": response.meta.suggestion,
            "hidden_uncertain_count": hidden_uncertain_count,
        },
    });
    // CORE-2 (2026-05-22) — forward disambiguation payload.
    if let Some(dis) = response.disambiguation.as_ref() {
        payload["disambiguation"] = json!(dis);
    }
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
