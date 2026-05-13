//! MCP method handlers. Each handler is a pure function taking the decoded
//! params and returning the result type — testable without stdio.

pub use crate::types::InitializeParams;
use crate::types::{
    Capabilities, InitializeResult, ServerInfo, ToolsCallParams, ToolsCallResult, ToolsCapability,
    ToolsListResult,
};
use crate::{MCP_PROTOCOL_VERSION, SERVER_NAME};

/// AS-015 — respond to `initialize`. Protocol version, server info, and
/// capabilities are all static; params.protocol_version is ignored for server
/// response (the server always advertises what it implements).
pub fn handle_initialize(_params: &InitializeParams) -> InitializeResult {
    InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION.to_string(),
        server_info: ServerInfo {
            name: SERVER_NAME.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        capabilities: Capabilities {
            tools: Some(ToolsCapability {
                list_changed: false,
            }),
        },
    }
}

/// AS-016 — respond to `tools/list`. Returns the static tool registry.
pub fn handle_tools_list() -> ToolsListResult {
    ToolsListResult {
        tools: crate::tools::registered_tools(),
    }
}

/// AS-017 — route a `tools/call` request. Returns `Ok(result)` on success, or
/// `Err(ga_core::Error)` for the MCP server to translate into a JSON-RPC
/// error via [`crate::error::to_jsonrpc_error`].
pub fn handle_tools_call(params: &ToolsCallParams) -> ga_core::Result<ToolsCallResult> {
    crate::tools::dispatch_tool_call(&params.name, &params.arguments)
}

/// Cluster F — ctx-aware `tools/call`. Required for tools that query the
/// graph (e.g. `ga_callers`). The stdio loop holds the Store and calls this
/// variant; the ctx-less form above is kept only for name/argument routing
/// tests.
///
/// v1.5 PR5 Staleness Phase C (sub-spec staleness S-003):
/// - `ga_reindex` bypasses the staleness gate unconditionally (AS-012)
/// - Other tools: pre-dispatch staleness check fires
///   - Fresh → dispatch normally (AS-007)
///   - Stale + `allow_stale: true` arg → dispatch + caller can read
///     `meta.stale` from `common.rs` annotation layer (AS-009)
///   - Stale without `allow_stale` → `Err(StaleIndex)` (AS-008)
///   - Degraded FS detection → dispatch (cannot reliably detect stale on
///     exotic FS; warn-only via meta.stale_check_degraded) (AS-010)
pub fn handle_tools_call_with_ctx(
    ctx: &crate::context::McpContext,
    params: &ToolsCallParams,
) -> ga_core::Result<ToolsCallResult> {
    // AS-012: ga_reindex MUST always be dispatchable so the agent can
    // recover from STALE_INDEX. Skip the gate unconditionally.
    if params.name == "ga_reindex" {
        return crate::tools::dispatch_tool_call_with_ctx(ctx, &params.name, &params.arguments);
    }

    // Read indexed_root_hash from metadata. Empty hash = "never committed"
    // sentinel — treat as fresh (no anchor to compare against, no drift
    // claim possible).
    let store_for_hash = ctx.store();
    let indexed_hex = &store_for_hash.metadata().indexed_root_hash;
    if indexed_hex.len() == 64 {
        // Decode hex → [u8; 32]. If the on-disk value is malformed,
        // treat as a degraded gate (don't fail-closed on metadata corrupt;
        // the cache lifecycle would have caught structural corruption).
        if let Some(indexed_bytes) = decode_root_hash(indexed_hex) {
            // AS-009: read allow_stale from args.
            let allow_stale = params
                .arguments
                .get("allow_stale")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            match ctx.staleness.check(&indexed_bytes) {
                Ok(result) => {
                    // AS-010: degraded FS → serve, do not fail closed.
                    // The annotation layer surfaces this via meta.stale_check_degraded.
                    if result.degraded {
                        return crate::tools::dispatch_tool_call_with_ctx(
                            ctx,
                            &params.name,
                            &params.arguments,
                        );
                    }
                    // AS-008: fail-closed default when stale + no opt-in.
                    if result.stale && !allow_stale {
                        return Err(ga_core::Error::StaleIndex {
                            indexed_root: indexed_hex.clone(),
                            current_root: hex_encode_32(&result.current_hash),
                            drift_detected_at: unix_now(),
                            dirty_paths: Vec::new(),
                        });
                    }
                    // S-004 AS-013/014 — Tier 1 fresh; run Tier 2
                    // BLAKE3 dirty-paths walk to catch content-only
                    // edits that didn't bump the bounded Merkle sample.
                    // Skipped under env opt-out (AS-017) or allow_stale.
                    if !result.stale && !allow_stale && !ctx.staleness.is_tier2_disabled() {
                        let dirty = tier2_dirty_paths_with_cache(ctx);
                        if !dirty.is_empty() {
                            return Err(ga_core::Error::StaleIndex {
                                indexed_root: indexed_hex.clone(),
                                current_root: hex_encode_32(&result.current_hash),
                                drift_detected_at: unix_now(),
                                dirty_paths: dirty
                                    .into_iter()
                                    .map(|p| p.to_string_lossy().replace('\\', "/").to_string())
                                    .collect(),
                            });
                        }
                    }
                    // AS-007 or AS-009: fresh OR (stale + allow_stale) → dispatch.
                }
                Err(_e) => {
                    // Staleness check itself errored (compute_root_hash
                    // failed — e.g. repo path vanished). Don't fail-closed
                    // since this is a degraded mode signal, not staleness
                    // proof. Fall through to dispatch.
                }
            }
        }
    }

    crate::tools::dispatch_tool_call_with_ctx(ctx, &params.name, &params.arguments)
}

/// Decode a 64-char lowercase hex string into 32 bytes. Returns None on
/// any parse failure — callers treat as "no anchor available".
fn decode_root_hash(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let byte_str = hex.get(i * 2..i * 2 + 2)?;
        out[i] = u8::from_str_radix(byte_str, 16).ok()?;
    }
    Some(out)
}

fn hex_encode_32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// v1.5 S-004 AS-013/014/015/016 — Tier 2 dirty-paths probe with
/// cache. Cache hit (1s TTL, valid `.git/index` mtime per AS-016) →
/// return cached set without walking. Miss → invoke
/// `ga_query::incremental::dirty_paths`, record + return. Internal
/// errors from the walk fall through as "empty dirty set" so we
/// degrade-open rather than fail-closed (mirrors AS-010 degraded
/// passthrough — internal gate errors must not block tools).
fn tier2_dirty_paths_with_cache(ctx: &crate::context::McpContext) -> Vec<std::path::PathBuf> {
    if let Some(cached) = ctx.staleness.tier2_lookup() {
        return cached;
    }
    let store = ctx.store();
    let repo_root = ctx.staleness.repo_root();
    match ga_query::incremental::dirty_paths(store.as_ref(), repo_root) {
        Ok(paths) => {
            ctx.staleness.record_tier2_result(paths.clone());
            paths
        }
        Err(e) => {
            tracing::debug!(error = %e, "Tier 2 dirty_paths probe failed; degrading open");
            // Record empty so we still hit cache for the next call
            // within TTL — don't keep re-erroring under load.
            ctx.staleness.record_tier2_result(Vec::new());
            Vec::new()
        }
    }
}
