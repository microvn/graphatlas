//! v1.1-M2 S-001 — `ga_risk` standalone risk tool (Layer 2).
//!
//! Composite formula per Tools-C2 (PINNED):
//!   `0.4·test_gap + 0.3·blast_radius + 0.15·blame_churn + 0.15·bug_correlation`
//!
//! The blame-mined dims (`blame_churn` + `bug_correlation`) are computed
//! by spawning `git log` against the indexed repo's working tree
//! (`store.metadata().repo_root`). When git is unavailable or the file
//! has no history, those dims contribute 0.0 — graceful degrade rather
//! than hard error per Tools-C1.

use super::common::{inject_common_meta, store_ctx_required_error};
use crate::context::McpContext;
use crate::types::{ContentBlock, ToolDescriptor, ToolsCallResult};
use ga_core::{Error, Result};
use ga_query::blame::GitLogMiner;
use ga_query::risk::{risk, RiskRequest};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;

pub(super) fn descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: "ga_risk".to_string(),
        description:
            "Composite risk score for refactor proposals. Given a `symbol` or `changed_files` \
             list, returns `score` ∈ [0,1], `level` (low|medium|high), and ranked `reasons`. \
             Composes test-coverage gap (40%), blast radius via callers/refs (30%), git-blame \
             churn (15%, commits/90d on the seed file), and bug-fix correlation (15%, % commits \
             matching `fix|bug|error|crash|regression`). Use BEFORE proposing a refactor: \
             gate-decision tool to triage risky changes."
                .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Seed symbol name (e.g., \"set_password\"). Mutually exclusive with `changed_files`."
                },
                "file": {
                    "type": "string",
                    "description": "Optional narrowing hint for ambiguous symbol names (Tools-C11 polymorphic resolution)."
                },
                "changed_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Repo-relative file paths whose union risk to compute. Returns `meta.per_file: {file: score}` breakdown alongside max-per-file `score`."
                }
            }
        }),
    }
}

fn validate_args(args: &Value) -> Result<RiskRequest> {
    let obj = args
        .as_object()
        .ok_or_else(|| Error::Other(anyhow::anyhow!("ga_risk: arguments must be a JSON object")))?;

    let symbol = obj
        .get("symbol")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let file = obj
        .get("file")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let changed_files = match obj.get("changed_files") {
        None => None,
        Some(v) => {
            let arr = v.as_array().ok_or_else(|| {
                Error::InvalidParams(
                    "ga_risk: `changed_files` must be an array of strings".to_string(),
                )
            })?;
            let mut out = Vec::with_capacity(arr.len());
            for entry in arr {
                let s = entry.as_str().ok_or_else(|| {
                    Error::InvalidParams(
                        "ga_risk: `changed_files` entries must be strings".to_string(),
                    )
                })?;
                out.push(s.to_string());
            }
            Some(out)
        }
    };

    if symbol.is_none() && changed_files.is_none() {
        return Err(Error::InvalidParams(
            "ga_risk: at least one of `symbol` or `changed_files` is required".to_string(),
        ));
    }
    if symbol.is_some() && changed_files.is_some() {
        return Err(Error::InvalidParams(
            "ga_risk: `symbol` and `changed_files` are mutually exclusive".to_string(),
        ));
    }

    Ok(RiskRequest {
        symbol,
        file_hint: file,
        changed_files,
        // Production MCP path leaves anchor_ref None → wall-clock semantics
        // for live repos. Bench harness sets it explicitly to fixture HEAD.
        anchor_ref: None,
    })
}

pub(super) fn ctxless(args: &Value) -> Result<ToolsCallResult> {
    validate_args(args)?;
    Err(store_ctx_required_error("ga_risk"))
}

pub(super) fn call(ctx: &McpContext, args: &Value) -> Result<ToolsCallResult> {
    let start = Instant::now();
    let req = validate_args(args)?;

    let repo_root: PathBuf = ctx.store().metadata().repo_root.clone().into();
    let miner = GitLogMiner::new(&repo_root);
    let response = risk(ctx.store().as_ref(), &miner, &req)?;

    let mut payload = json!({
        "tool": "ga_risk",
        "score": response.score,
        "level": response.level,
        "reasons": response.reasons,
        "meta": {
            "per_dim": response.meta.per_dim,
            "per_file": response.meta.per_file,
        },
    });
    inject_common_meta(&mut payload, ctx, start);
    Ok(ToolsCallResult {
        content: vec![ContentBlock::Json { json: payload }],
        is_error: false,
    })
}
