//! Tool registry. One submodule per MCP tool keeps each descriptor +
//! validator + dispatcher co-located and bounded (<100 LoC each). The
//! top-level `registered_tools` / `dispatch_tool_call[_with_ctx]` stay at
//! `crate::tools::` for handlers.rs + external tests.

mod architecture;
mod bridges;
mod callees;
mod callers;
mod common;
mod dead_code;
mod file_summary;
mod hubs;
mod impact;
mod importers;
mod large_functions;
mod minimal_context;
mod reindex;
mod rename_safety;
mod risk;
mod symbols;

use crate::context::McpContext;
use crate::types::{ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use serde_json::Value;

/// AS-016 entry point — the static tool list exposed via `tools/list`.
pub fn registered_tools() -> Vec<ToolDescriptor> {
    vec![
        callers::descriptor(),
        callees::descriptor(),
        importers::descriptor(),
        symbols::descriptor(),
        file_summary::descriptor(),
        impact::descriptor(),
        risk::descriptor(),
        minimal_context::descriptor(),
        dead_code::descriptor(),
        rename_safety::descriptor(),
        architecture::descriptor(),
        hubs::descriptor(),
        bridges::descriptor(),
        large_functions::descriptor(),
        reindex::descriptor(),
    ]
}

/// AS-017 dispatcher — route by tool name. Ctx-less form is retained for
/// tests that exercise name-routing only (unknown tool, missing args); it
/// returns a typed error for any tool needing Store access.
pub fn dispatch_tool_call(name: &str, args: &Value) -> Result<ToolsCallResult> {
    match name {
        "ga_callers" => callers::ctxless(args),
        "ga_callees" => callees::ctxless(args),
        "ga_importers" => importers::ctxless(args),
        "ga_symbols" => symbols::ctxless(args),
        "ga_file_summary" => file_summary::ctxless(args),
        "ga_impact" => impact::ctxless(args),
        "ga_risk" => risk::ctxless(args),
        "ga_minimal_context" => minimal_context::ctxless(args),
        "ga_dead_code" => dead_code::ctxless(args),
        "ga_rename_safety" => rename_safety::ctxless(args),
        "ga_architecture" => architecture::ctxless(args),
        "ga_hubs" => hubs::ctxless(args),
        "ga_bridges" => bridges::ctxless(args),
        "ga_large_functions" => large_functions::ctxless(args),
        "ga_reindex" => reindex::ctxless(args),
        other => Err(unknown_tool_error(other)),
    }
}

/// Ctx-aware dispatch — route + execute against the live Store.
pub fn dispatch_tool_call_with_ctx(
    ctx: &McpContext,
    name: &str,
    args: &Value,
) -> Result<ToolsCallResult> {
    match name {
        "ga_callers" => callers::call(ctx, args),
        "ga_callees" => callees::call(ctx, args),
        "ga_importers" => importers::call(ctx, args),
        "ga_symbols" => symbols::call(ctx, args),
        "ga_file_summary" => file_summary::call(ctx, args),
        "ga_impact" => impact::call(ctx, args),
        "ga_risk" => risk::call(ctx, args),
        "ga_minimal_context" => minimal_context::call(ctx, args),
        "ga_dead_code" => dead_code::call(ctx, args),
        "ga_rename_safety" => rename_safety::call(ctx, args),
        "ga_architecture" => architecture::call(ctx, args),
        "ga_hubs" => hubs::call(ctx, args),
        "ga_bridges" => bridges::call(ctx, args),
        "ga_large_functions" => large_functions::call(ctx, args),
        "ga_reindex" => reindex::call(ctx, args),
        other => Err(unknown_tool_error(other)),
    }
}

fn unknown_tool_error(name: &str) -> Error {
    Error::Other(anyhow::anyhow!(
        "unknown tool: {name} (supported: ga_callers, ga_callees, ga_importers, ga_symbols, ga_file_summary, ga_impact, ga_risk, ga_minimal_context, ga_dead_code, ga_rename_safety, ga_architecture, ga_hubs, ga_bridges, ga_large_functions, ga_reindex)"
    ))
}
