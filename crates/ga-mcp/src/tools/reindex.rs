//! v1.5 PR6 — `ga_reindex` MCP tool (full-rebuild MVP).
//!
//! Spec: `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-tool.md`
//! S-001 (descriptor + input validation) + S-004 (per-repo mutex registry).
//!
//! **PR6 MVP scope** — ships:
//! - Tool descriptor + `tools/list` registration (AS-001)
//! - Input schema validation for `mode` arg (AS-002)
//! - Bench fixture path refusal (AS-004) — defense-in-depth with
//!   `prepare_store_for_mcp` boot guard
//! - Per-repo serialization mutex registry hookup (AS-009/010)
//!
//! **PR6.1b** wires the actual rebuild (R1b-S002.AS-003):
//! `ctx.rebuild_via(|store| store.reindex_in_place + build_index +
//! commit_in_place)`. Closure failure → [`Error::ReindexBuildFailed`]
//! (-32012). Active readers refcount > 1 → [`Error::StoreBusy`] (-32013).
//!
//! **Still deferred** (`## Not in Scope` in tool.md):
//! - Tombstone protocol (REBUILDING.tombstone) — AS-003 steps 4+8
//! - Cross-process flock + ALREADY_REINDEXING (-32014) — AS-011
//! - 200ms post-success cooldown / debounce — AS-006

use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use serde_json::{json, Value};

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_reindex".to_string(),
        description: "Rebuild the code graph from current repo state. \
                      Use after edits to refresh staleness (caller will otherwise \
                      receive STALE_INDEX errors on subsequent tool calls). \
                      `mode: \"full\"` (default) performs a full rebuild; \
                      `mode: \"auto\"` reserves the incremental path for Phase E."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["auto", "full"],
                    "description": "Reindex strategy. Defaults to \"full\". \
                                    \"auto\" reserved for Phase E incremental pipeline."
                },
                "correlation_id": {
                    "type": "string",
                    "description": "Optional UUID to correlate this reindex with \
                                    upstream agent logs. Server emits its own ID \
                                    if absent (see tracing reindex_span)."
                }
            }
        }),
    }
}

/// Mode argument variants per AS-002. `Auto` is reserved for the Phase E
/// incremental pipeline (PR9); PR6 treats both modes as `Full` since
/// incremental is out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReindexMode {
    Auto,
    Full,
}

/// Validate the tool's input arguments. AS-002: invalid `mode` → -32602.
pub(super) fn validate_args(args: &Value) -> Result<ReindexMode> {
    match args.get("mode") {
        None => Ok(ReindexMode::Full),
        Some(Value::String(s)) => match s.as_str() {
            "auto" => Ok(ReindexMode::Auto),
            "full" => Ok(ReindexMode::Full),
            other => Err(Error::InvalidParams(format!(
                "ga_reindex: `mode` must be \"auto\" or \"full\" (got {other:?})"
            ))),
        },
        Some(other) => Err(Error::InvalidParams(format!(
            "ga_reindex: `mode` must be a string (got {other})"
        ))),
    }
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    // Validation still runs in the ctxless path so test harnesses can
    // verify schema rejection without spinning up a Store.
    validate_args(args)?;
    Err(Error::Other(anyhow::anyhow!(
        "ga_reindex requires McpContext — call via the rmcp dispatch path"
    )))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let mode = validate_args(args)?;

    // AS-004: refuse on bench fixture paths (defense-in-depth — boot-time
    // guard `prepare_store_for_mcp::is_bench_fixture_path` should already
    // have refused, but tool-level refusal protects callers that may have
    // bypassed boot).
    let repo_root = std::path::PathBuf::from(&ctx.store().metadata().repo_root);
    if is_bench_fixture_path(&repo_root) {
        return Err(Error::Other(anyhow::anyhow!(
            "ga_reindex refused: bench fixture path detected ({}). \
             Reindex would corrupt M1/M2/M3 gates.",
            repo_root.display()
        )));
    }

    // AS-009/010: acquire per-repo serialization mutex. Cross-repo
    // reindexes use distinct Mutex<()> instances (proven by the
    // `reindex_lock_for` registry tests in tests/reindex_tool.rs).
    let cache_dir = ctx.store().layout().dir().to_path_buf();
    let lock_arc = ctx.reindex_lock_for(&cache_dir);
    let _guard = lock_arc.lock().expect("per-repo reindex mutex");

    // PR6.1d AS-006 — post-success cooldown short-circuit. Held INSIDE the
    // per-repo mutex so two concurrent in-process callers can't both pass
    // the check (Mutex serializes them; the second one observes the first
    // one's record_reindex_success and short-circuits).
    ctx.check_reindex_cooldown(&cache_dir)?;

    // R1b-S002.AS-003 — actual close-rm-init rebuild via the RwLock-wrapped
    // store cell. We snapshot generation_before from the live store *before*
    // surrendering it to rebuild_via (the closure consumes it by value).
    let gen_before = ctx.store().metadata().graph_generation;
    let started = std::time::Instant::now();
    let mut files_indexed: u64 = 0;
    let new_store = ctx.rebuild_via(|store| {
        let repo_root_inner =
            std::path::PathBuf::from(&store.metadata().repo_root);
        let mut fresh = store
            .reindex_in_place(&repo_root_inner)
            .map_err(|e| Error::Other(anyhow::anyhow!("reindex_in_place: {e}")))?;
        let stats = ga_query::indexer::build_index(&fresh, &repo_root_inner)
            .map_err(|e| Error::Other(anyhow::anyhow!("build_index: {e}")))?;
        files_indexed = stats.files as u64;
        fresh
            .commit_in_place()
            .map_err(|e| Error::Other(anyhow::anyhow!("commit_in_place: {e}")))?;
        Ok(fresh)
    })?;
    let took_ms = started.elapsed().as_millis() as u64;
    let gen_after = new_store.metadata().graph_generation;
    let new_root_hash = new_store.metadata().indexed_root_hash.clone();

    // PR6.1d AS-006 — arm the cooldown window so a rapid follow-up
    // ga_reindex call (e.g. from FS watcher + hook installer both firing
    // on the same edit) short-circuits with -32014 AlreadyReindexing.
    ctx.record_reindex_success(&cache_dir);

    let payload = json!({
        "tool": "ga_reindex",
        "mode": match mode {
            ReindexMode::Auto => "auto",
            ReindexMode::Full => "full",
        },
        "reindexed": true,
        "took_ms": took_ms,
        "files_indexed": files_indexed,
        "graph_generation_before": gen_before,
        "graph_generation_after": gen_after,
        "new_root_hash": new_root_hash,
    });
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}


/// v1.5 PR6 AS-004 — local copy of the bench-fixture path detector used
/// at MCP boot (`src/mcp_cmd.rs::is_bench_fixture_path`). Duplicated here
/// because ga-mcp can't depend on the graphatlas binary crate; both
/// copies share the same intent + same fallback semantics.
fn is_bench_fixture_path(p: &std::path::Path) -> bool {
    let canonical = std::fs::canonicalize(p).ok();
    let probe = canonical.as_deref().unwrap_or(p);
    let s = probe.to_string_lossy().replace('\\', "/");
    s.contains("/benches/fixtures/")
}
