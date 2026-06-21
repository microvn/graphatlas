//! Tools S-001 — `ga_callers` MCP tool.

use super::common::{
    inject_common_meta, store_ctx_required_error, validate_symbol_file_args,
    wants_include_uncertain,
};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::Result;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_callers".to_string(),
        description: "**Use instead of grep when** the user asks `who calls X`, `callers of X`, \
             `references to X`, `where is X used`, or wants the blast radius of a symbol. \
             Reads typed CALL + REFERENCES edges from the indexed code graph — grep matches \
             comments + strings + unrelated occurrences and inflates results. Each entry has \
             `kind: \"call\"` (direct invocation) or `kind: \"reference\"` (symbol held by \
             value — dispatch map, callback array, shorthand property), plus file, symbol \
             name, definition line, and call-site / reference-site line. Polymorphic dispatch \
             is resolved per Tools-C11."
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
                },
                "format": {
                    "type": "string",
                    "enum": ["json", "markdown"],
                    "description": "Output format. `markdown` is ~50% cheaper in tokens; recommended for LLM-agent callers. Default: `json` (backward compat)."
                },
                "include_uncertain": {
                    "type": "boolean",
                    "description": "When `file:` hint is set on a multi-def symbol, also surface polymorphic conf-0.6 entries from other defs. Default: false (drop noise, keep only conf 1.0)."
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
    let mut response = ga_query::callers(ctx.try_store()?.as_ref(), symbol, file)?;

    // P1.3 (2026-05-22) — drop conf < 1.0 entries unless caller opts in.
    // Surfaces only the exact-match callers narrowed by the `file:` hint;
    // polymorphic same-name-from-other-file entries become noise.
    let include_uncertain = wants_include_uncertain(args);
    let hidden_uncertain_count = if include_uncertain {
        0
    } else {
        let before = response.callers.len();
        response.callers.retain(|c| c.confidence >= 1.0);
        (before - response.callers.len()) as u64
    };

    // P3.1 (2026-05-22) — compact mode default. Aggregate per-call-site
    // entries into 1 entry per (caller, file) with `call_sites` array. Opt-out
    // via `verbosity: "flat"` for legacy / programmatic consumers that need
    // every per-site row.
    let use_compact = crate::compact::wants_compact(args);

    // P1.5 (2026-05-22) — Markdown opt-in for LLM-agent callers.
    if crate::markdown::wants_markdown(args) {
        let text = if use_compact && response.disambiguation.is_none() {
            let compact = crate::compact::compact_callers(response.callers);
            let mut s = format!(
                "## Callers of {symbol} ({} callers, {} sites)\n",
                compact.len(),
                compact.iter().map(|c| c.call_site_count).sum::<u32>()
            );
            for c in &compact {
                s.push_str(&crate::compact::render_compact_caller_md(c));
                s.push('\n');
            }
            s
        } else {
            crate::markdown::render_callers(
                symbol,
                &response.callers,
                response.disambiguation.as_ref(),
            )
        };
        let _ = start;
        return Ok(ToolsCallResult {
            content: vec![ContentBlock::Text { text }],
            is_error: false,
        });
    }

    // JSON path — compact callers when no ambiguity AND default verbosity.
    let callers_payload = if use_compact && response.disambiguation.is_none() {
        json!(crate::compact::compact_callers(response.callers))
    } else {
        json!(response.callers)
    };

    let mut payload = json!({
        "tool": "ga_callers",
        "symbol": symbol,
        "file": file,
        "callers": callers_payload,
        "meta": {
            "symbol_found": response.meta.symbol_found,
            "suggestion": response.meta.suggestion,
            "hidden_uncertain_count": hidden_uncertain_count,
        },
    });
    // CORE-2 (2026-05-22) — forward ambiguity-first payload when present so the
    // MCP client can disambiguate before retry.
    if let Some(dis) = response.disambiguation.as_ref() {
        payload["disambiguation"] = json!(dis);
    }
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
