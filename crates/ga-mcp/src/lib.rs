//! MCP server scaffold. S-006 shipped dispatch machinery; infra:S-003
//! (v1.1-M0) wires it to `rmcp 1.5` stdio transport so `graphatlas mcp`
//! runs end-to-end against real clients (Claude Code, Cursor, Cline).
//!
//! Layering:
//!   lib.rs  — public entry points (run_stdio / serve_with_store) +
//!             the ServerHandler impl that bridges rmcp callbacks into
//!             handlers::dispatch.
//!   handlers.rs / tools/ — transport-agnostic request dispatch. rmcp
//!             wraps these; neither types nor tool descriptors change.

pub mod context;
pub mod error;
pub mod handlers;
pub mod telemetry;
pub mod tools;
pub mod types;
pub mod watcher;

pub use error::{to_jsonrpc_error, JsonRpcError};

use std::sync::Arc;

use ga_core::{Error as GaError, Result as GaResult};
use ga_index::Store;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorCode, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, Tool,
    ToolsCapability,
};
use rmcp::service::{RequestContext, RoleServer, ServiceExt};
use rmcp::{ErrorData as McpError, ServerHandler};

/// MCP spec version per Foundation R37 + AS-015. Full date, not YYYY-MM.
/// <https://modelcontextprotocol.io/specification/2025-11-25>.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

pub const SERVER_NAME: &str = "graphatlas";

/// rmcp-backed handler. One instance per server lifetime. Holds a
/// long-lived `McpContext` so per-repo state (reindex mutex registry +
/// 200ms cooldown timestamps from PR6.1d) is shared across tool calls
/// AND with the v1.5 PR8 Layer 1 `.git/` watcher when it's wired in
/// from `cmd_mcp`.
pub struct GaServerHandler {
    ctx: context::McpContext,
}

impl GaServerHandler {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            ctx: context::McpContext::new(store),
        }
    }

    /// PR8 — construct with a pre-built `McpContext` so callers wiring
    /// the L1 watcher can share the same context (per-repo mutex +
    /// cooldown) between the watcher's reindex dispatch and the
    /// tool-call handler.
    pub fn with_ctx(ctx: context::McpContext) -> Self {
        Self { ctx }
    }

    /// Read-only handle to the underlying context — used by the L1
    /// watcher to dispatch `rebuild_via` against the same per-repo
    /// state as the tool-call path.
    pub fn ctx(&self) -> &context::McpContext {
        &self.ctx
    }

    fn rmcp_tools(&self) -> Vec<Tool> {
        tools::registered_tools()
            .into_iter()
            .map(|d| {
                let schema = d.input_schema.as_object().cloned().unwrap_or_default();
                let mut tool = Tool::default();
                tool.name = d.name.into();
                tool.description = Some(d.description.into());
                tool.input_schema = Arc::new(schema);
                tool
            })
            .collect()
    }
}

impl ServerHandler for GaServerHandler {
    fn get_info(&self) -> InitializeResult {
        let mut caps = ServerCapabilities::default();
        let mut tools_cap = ToolsCapability::default();
        tools_cap.list_changed = Some(false);
        caps.tools = Some(tools_cap);

        let mut impl_info = Implementation::default();
        impl_info.name = SERVER_NAME.into();
        impl_info.version = env!("CARGO_PKG_VERSION").into();

        let mut result = InitializeResult::new(caps);
        result.protocol_version = ProtocolVersion::V_2025_11_25;
        result.server_info = impl_info;
        result
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = ListToolsResult::default();
        result.tools = self.rmcp_tools();
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.as_ref();
        let args = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);

        let ctx = self.ctx.clone();
        let started = std::time::Instant::now();
        // v1.5 PR5 — route through the staleness-gate-aware entry point
        // so STALE_INDEX (-32010), allow_stale opt-in, degraded-FS
        // passthrough, and ga_reindex bypass all fire on the real
        // rmcp transport. Direct `dispatch_tool_call_with_ctx` skips
        // the gate, which is a regression `tests/smoke_reindex_subprocess.rs`
        // pins against.
        let params = crate::types::ToolsCallParams {
            name: name.to_string(),
            arguments: args.clone(),
        };
        let dispatch_result = handlers::handle_tools_call_with_ctx(&ctx, &params);
        let elapsed = started.elapsed();

        match dispatch_result {
            Ok(result) => {
                // Pre-render text payloads once; reuse for both the rmcp
                // response and (if enabled) the telemetry log. Response
                // bytes are bit-identical to the prior implementation —
                // telemetry is read-only.
                let texts: Vec<String> = result
                    .content
                    .into_iter()
                    .map(|block| match block {
                        crate::types::ContentBlock::Text { text } => text,
                        crate::types::ContentBlock::Json { json } => json.to_string(),
                    })
                    .collect();
                if let Some(t) = telemetry::Telemetry::global() {
                    let joined = texts.join("\n");
                    t.log_call(name, &args, &joined, result.is_error, None, elapsed);
                }
                let content: Vec<Content> = texts.into_iter().map(Content::text).collect();
                Ok(if result.is_error {
                    CallToolResult::error(content)
                } else {
                    CallToolResult::success(content)
                })
            }
            Err(e) => {
                if let Some(t) = telemetry::Telemetry::global() {
                    let msg = format!("{e:#}");
                    t.log_call(name, &args, "", true, Some(&msg), elapsed);
                }
                Err(ga_core_error_to_mcp(&e))
            }
        }
    }
}

/// Phase A review [H-1]: map `ga_core::Error` variants to rmcp `McpError`
/// preserving the JSON-RPC code taxonomy from Foundation-C5 + AS-023.
/// `ga_core::Error::jsonrpc_code()` is the single source of truth for
/// the numeric code; we wrap that in rmcp's error-data envelope.
fn ga_core_error_to_mcp(err: &GaError) -> McpError {
    let code = ErrorCode(err.jsonrpc_code());
    McpError::new(code, format!("{err}"), None)
}

/// Start an rmcp server over the supplied duplex streams. Used by
/// integration tests (tokio duplex pipe) and by `run_stdio()` (stdin +
/// stdout). Returns when the transport closes (EOF) or the client cancels.
pub async fn serve_with_store<R, W>(read: R, write: W, store: Arc<Store>) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    serve_with_ctx(read, write, context::McpContext::new(store)).await
}

/// PR8 variant — caller passes a pre-built `McpContext` so the same
/// per-repo state (reindex mutex registry, cooldown clocks) is shared
/// with the L1 watcher spawned in parallel.
pub async fn serve_with_ctx<R, W>(
    read: R,
    write: W,
    ctx: context::McpContext,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let handler = GaServerHandler::with_ctx(ctx);
    let running = handler.serve((read, write)).await?;
    running.waiting().await?;
    Ok(())
}

/// Stdio loop — reads stdin / writes stdout under the rmcp transport.
/// `store` opened against the current repo by the caller (typically
/// `src/main.rs::cmd_mcp`). This function blocks the caller by spinning
/// up a tokio current-thread runtime; the `graphatlas mcp` subcommand is
/// inherently single-connection so an async entry point is unnecessary.
pub fn run_stdio(store: Arc<Store>) -> GaResult<()> {
    run_stdio_with_ctx(context::McpContext::new(store))
}

/// PR8 variant of [`run_stdio`] for callers that built their own
/// `McpContext` (typically to share state with the L1 watcher).
pub fn run_stdio_with_ctx(ctx: context::McpContext) -> GaResult<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| GaError::Other(anyhow::anyhow!("tokio runtime: {e}")))?;
    rt.block_on(async move {
        serve_with_ctx(tokio::io::stdin(), tokio::io::stdout(), ctx)
            .await
            .map_err(|e| GaError::Other(anyhow::anyhow!("serve_with_ctx: {e}")))
    })
}
