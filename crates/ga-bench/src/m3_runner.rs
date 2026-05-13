//! M3 gate runner — `graphatlas-v1.1-bench.md` S-001.
//!
//! Mirrors [`crate::m2_runner`] surface but targets the V1.1 decision-support
//! tools (`ga_dead_code`, `ga_rename_safety`, `ga_minimal_context`,
//! `ga_architecture`). `ga_risk` measurement is reserved for Phase 3 — the
//! `risk` UC is intentionally absent from the supported list and produces a
//! clear "DEFERRED to Phase 3" error if requested.
//!
//! Public contract pinned by AS-001:
//! ```text
//! pub fn run(config: M3GateConfig) -> Result<Vec<M3LeaderboardRow>, BenchError>
//! ```
//! Sync (mirror m2_runner sync style); takes ownership of config; returns
//! aggregate rows or error. Internal helpers stay private.
//!
//! Per-UC scoring loops (the actual `ga_*` invocations) live in sibling
//! modules to keep this file focused on dispatch + types:
//!   - `crate::m3_minimal_context` — Hmc-budget loop (S-004 cycle B)

use crate::BenchError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Valid M3 UCs (Phase 1+2 measurable scope). `risk` is DEFERRED to Phase 3
/// per spec §"Not in Scope" — see `graphatlas-v1.1-bench-risk.md` future
/// sub-spec. Keep this list in sync with bench-CLI dispatch + AS-002 error.
pub const M3_UC_NAMES: &[&str] = &[
    "dead_code",
    "rename_safety",
    "minimal_context",
    "architecture",
    "risk",
    "hubs",
];

/// Per-tool acceptance status surfaced on each leaderboard row.
/// `Tautological` flags Spearman/F1 ≥ 0.95 on Ha-import-edge per AS-020.
/// `Deferred` reserved for `risk` UC rows in Phase 1+2 partial leaderboards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SpecStatus {
    Pass,
    Fail,
    Tautological,
    Deferred,
}

#[derive(Debug, Clone, Serialize)]
pub struct M3GateConfig {
    pub uc: String,
    pub fixture: String,
    pub retrievers: Vec<String>,
    pub gate: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct M3LeaderboardRow {
    pub retriever: String,
    pub fixture: String,
    pub uc: String,
    pub score: f64,
    pub secondary_metrics: BTreeMap<String, f64>,
    pub spec_status: SpecStatus,
    pub spec_target: f64,
    pub p95_latency_ms: u64,
}

/// Entry point for M3 gate runs.
///
/// Phase 1a stub: validates UC + returns empty rows when retriever set is
/// empty (smoke). Real per-UC dispatch lands in S-004 (Hmc-budget) once the
/// Hmc-budget rule + adapter are wired. AS-001 cycle 1 only proves the
/// surface compiles and rejects unknown UCs cleanly.
pub fn run(config: M3GateConfig) -> Result<Vec<M3LeaderboardRow>, BenchError> {
    if !M3_UC_NAMES.contains(&config.uc.as_str()) {
        return Err(BenchError::UnknownM3Uc(config.uc));
    }
    // Phase 1a stub: real per-UC scoring lands in S-004+. Returning empty rows
    // for an empty retriever set proves the surface accepts a valid config.
    if config.retrievers.is_empty() {
        return Ok(Vec::new());
    }
    // For Phase 1a we don't have any rule wired yet — return empty until
    // S-004 (Hmc-budget) lands. This keeps the gate honest: no retriever
    // gets a fake `PASS` row before its rule exists.
    Ok(Vec::new())
}

// S-004 cycle B — `ScoreOpts`, `score_minimal_context`, and the constants
// live in `crate::m3_minimal_context` so this file stays focused on
// dispatch + shared types. Re-exported so existing
// `use ga_bench::m3_runner::{ScoreOpts, score_minimal_context}` imports
// continue to compile.
pub use crate::m3_architecture::{score_architecture, ARCHITECTURE_SPEC_TARGET};
pub use crate::m3_dead_code::{score_dead_code, DEAD_CODE_SPEC_TARGET};
pub use crate::m3_hubs::{score_hubs, HUBS_SPEC_TARGET};
pub use crate::m3_minimal_context::{
    score_minimal_context, ScoreOpts, MINIMAL_CONTEXT_BUDGET, MINIMAL_CONTEXT_SPEC_TARGET,
};
pub use crate::m3_rename_safety::{score_rename_safety, RENAME_POLY_TARGET, RENAME_UNIQUE_TARGET};
pub use crate::m3_risk::{score_risk, RISK_F1_TARGET};

/// AS-001/002/004 — testable CLI core. `main.rs` does arg parsing only and
/// passes through here so behaviour is unit-testable without spawning a
/// subprocess. Returns the desired process exit code on the success path
/// (0 = all PASS or no rows; 1 = at least one FAIL; 2 = unknown UC) and a
/// `BenchError::UnknownM3Uc` on the unknown-UC path so the CLI can map it
/// to exit 2 + stderr.
#[derive(Debug, Clone)]
pub struct M3CliOutcome {
    pub exit_code: i32,
    pub leaderboard_path: std::path::PathBuf,
    pub rows: Vec<M3LeaderboardRow>,
}

pub fn run_m3_cli(
    output_dir: &Path,
    uc: &str,
    fixture: &str,
    retrievers: &[String],
) -> Result<M3CliOutcome, BenchError> {
    if !M3_UC_NAMES.contains(&uc) {
        return Err(BenchError::UnknownM3Uc(uc.to_string()));
    }

    // All four Phase 1+2 UCs dispatch through real scoring loops when a
    // retriever is requested. Empty retriever list ⇒ stub returns empty
    // rows (smoke surface only).
    let rows = if !retrievers.is_empty() {
        let repo_root = std::env::current_dir()?;
        let opts = ScoreOpts {
            fixture_name: fixture.to_string(),
            fixture_dir: repo_root.join("benches").join("fixtures").join(fixture),
            cache_root: repo_root
                .join(".graphatlas-bench-cache")
                .join(format!("m3-{uc}"))
                .join(fixture),
            retrievers: retrievers.to_vec(),
            gt_path: None,
            split: None,
        };
        match uc {
            "minimal_context" => score_minimal_context(&opts)?,
            "dead_code" => score_dead_code(&opts)?,
            "rename_safety" => score_rename_safety(&opts)?,
            "architecture" => score_architecture(&opts)?,
            "risk" => score_risk(&opts)?,
            "hubs" => score_hubs(&opts)?,
            _ => unreachable!("UC validated against M3_UC_NAMES above"),
        }
    } else {
        let cfg = M3GateConfig {
            uc: uc.to_string(),
            fixture: fixture.to_string(),
            retrievers: retrievers.to_vec(),
            gate: "m3".to_string(),
        };
        run(cfg)?
    };

    // AS-013.T1 — rule + bias come from the rule registry. Only Hmc-budget
    // (S-004) ships in Phase 1a; the other three UCs still flag "pending
    // S-XXX" until S-005/006/007 land their rule files.
    let (rule_name, policy_bias_owned): (String, String) = {
        use crate::gt_gen::GtRule;
        match uc {
            "minimal_context" => {
                let rule = crate::gt_gen::hmc_gitmine::HmcGitmine::default();
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            "dead_code" => {
                let rule = crate::gt_gen::hd_ast::HdAst;
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            "rename_safety" => {
                let rule = crate::gt_gen::hrn_static::HrnStatic;
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            "architecture" => {
                let rule = crate::gt_gen::ha_import_edge::HaImportEdge;
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            "risk" => {
                let rule = crate::gt_gen::hr_text::HrText;
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            "hubs" => {
                let rule = crate::gt_gen::hh_gitmine::HhGitmine::default();
                (rule.id().to_string(), rule.policy_bias().to_string())
            }
            _ => ("<unknown>".to_string(), String::new()),
        }
    };

    let md = render_leaderboard_md(uc, fixture, &rule_name, &policy_bias_owned, &rows);
    std::fs::create_dir_all(output_dir)?;
    let leaderboard_path = output_dir.join(format!("m3-{uc}-{fixture}-leaderboard.md"));
    std::fs::write(&leaderboard_path, md)?;

    let exit_code = if has_failures(&rows) { 1 } else { 0 };
    Ok(M3CliOutcome {
        exit_code,
        leaderboard_path,
        rows,
    })
}

/// AS-011.T2 — verify a `<path>.sha256` sidecar against the live file.
///
/// Returns `Ok(())` if the file's actual sha256 matches the sidecar's
/// recorded digest. Returns `BenchError` (not panic — caller decides
/// whether to bubble or panic) on mismatch / missing sidecar / I/O error.
/// Spec calls for a panic at the rule layer; we expose this as a `Result`
/// helper so tests can assert on the error message instead of catching
/// panics — the rule wrapper panics via `.expect(...)`.
pub fn verify_sha256_sidecar(file_path: &Path) -> Result<(), BenchError> {
    let sidecar_path = file_path.with_file_name(format!(
        "{}.sha256",
        file_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
    ));
    if !sidecar_path.exists() {
        return Err(BenchError::Other(anyhow::anyhow!(
            "sha256 sidecar missing at {}: dataset cannot be verified. \
             Regenerate via `--refresh-gt` or restore the sidecar from git.",
            sidecar_path.display()
        )));
    }
    let sidecar_text = std::fs::read_to_string(&sidecar_path)?;
    let sidecar: serde_json::Value = serde_json::from_str(&sidecar_text)
        .map_err(|e| BenchError::Other(anyhow::anyhow!("sidecar JSON malformed: {e}")))?;
    let expected = sidecar
        .get("sha256")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            BenchError::Other(anyhow::anyhow!(
                "sidecar at {} missing `sha256` field",
                sidecar_path.display()
            ))
        })?
        .to_lowercase();

    let payload = std::fs::read(file_path)?;
    let actual = format!("{:x}", Sha256::digest(&payload));

    if actual != expected {
        return Err(BenchError::Other(anyhow::anyhow!(
            "{}: sha256 mismatch — expected {expected}, got {actual}. \
             Dataset is corrupt; check git lfs / merge conflicts, or regenerate \
             the sidecar via `--refresh-gt`.",
            file_path.display()
        )));
    }
    Ok(())
}

/// AS-003 — atomic GT write with sha256 sidecar.
///
/// Writes `payload` to `<final>.tmp`, fsyncs, atomic-renames to `<final>`,
/// then writes `<final>.sha256` carrying `{ task_count, sha256 }`. Concurrent
/// writers serialize at the rename: the OS guarantees atomicity, so the final
/// file is always exactly one writer's payload — never interleaved bytes.
///
/// `task_count` is caller-supplied because the GT shape is rule-specific
/// (Hd-ast counts symbols, Hmc-budget counts tasks, etc.) — keeping the
/// helper payload-agnostic avoids parsing JSON twice.
pub fn write_gt_atomic(
    final_path: &Path,
    payload: &[u8],
    task_count: usize,
) -> std::io::Result<()> {
    let parent = final_path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "GT path has no parent")
    })?;
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
    }

    // Per-thread tmp suffix — multiple in-flight writers must not stomp the
    // same `<final>.tmp` mid-write (which would defeat atomicity). The OS
    // rename still serializes; this just keeps tmp blobs distinct.
    let pid = std::process::id();
    let tid_hash: u64 = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&std::thread::current().id(), &mut h);
        std::hash::Hasher::finish(&h)
    };
    let tmp_path = final_path.with_file_name(format!(
        "{}.tmp.{pid}.{tid_hash:x}",
        final_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("gt.json")
    ));

    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(payload)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, final_path)?;

    let hex = format!("{:x}", Sha256::digest(payload));
    let sidecar_path = final_path.with_file_name(format!(
        "{}.sha256",
        final_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("gt.json")
    ));
    let sidecar = serde_json::json!({
        "task_count": task_count,
        "sha256": hex,
    });
    let sidecar_tmp = sidecar_path.with_extension(format!("sha256.tmp.{pid}.{tid_hash:x}"));
    {
        let mut f = std::fs::File::create(&sidecar_tmp)?;
        f.write_all(serde_json::to_string_pretty(&sidecar).unwrap().as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&sidecar_tmp, &sidecar_path)?;
    Ok(())
}

/// AS-004 — does this leaderboard contain any hard failure?
/// `Tautological` is a warning per AS-020, not a hard fail; `Deferred` is the
/// `risk` UC stub. CLI exit code 1 hooks on `true` only when at least one
/// retriever × UC × fixture row is `Fail`.
pub fn has_failures(rows: &[M3LeaderboardRow]) -> bool {
    rows.iter().any(|r| r.spec_status == SpecStatus::Fail)
}

/// AS-004 / AS-013 — render M3 leaderboard markdown.
///
/// Header carries `**Rule:** <rule_name>` and `**Policy bias:** <bias>` once
/// (single source of truth — `GtRule::policy_bias()` is the only producer).
/// Failing rows get `**FAIL**` bold prefix in the row marker column. Footer
/// summarises pass/fail counts. Markdown is emitted unconditionally for
/// audit; CI exit teeth live in `has_failures` + the CLI dispatcher.
pub fn render_leaderboard_md(
    uc: &str,
    fixture: &str,
    rule_name: &str,
    policy_bias: &str,
    rows: &[M3LeaderboardRow],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# M3 Gate — `{uc}` on `{fixture}`\n\n"));
    out.push_str(&format!("**Rule:** {rule_name}\n\n"));
    out.push_str(&format!("**Policy bias:** {policy_bias}\n\n"));

    out.push_str("| Status | Retriever | Score | Spec target | p95 latency (ms) |\n");
    out.push_str("|---|---|---|---|---|\n");
    let mut pass = 0usize;
    let mut fail = 0usize;
    for r in rows {
        let (marker, count) = match r.spec_status {
            SpecStatus::Pass => ("PASS", &mut pass),
            SpecStatus::Fail => ("**FAIL**", &mut fail),
            SpecStatus::Tautological => ("**TAUTOLOGY-SUSPECT**", &mut pass),
            SpecStatus::Deferred => ("DEFERRED", &mut pass),
        };
        *count += 1;
        out.push_str(&format!(
            "| {marker} | {} | {:.3} | {:.3} | {} |\n",
            r.retriever, r.score, r.spec_target, r.p95_latency_ms,
        ));
    }
    if rows.is_empty() {
        out.push_str("| _no rows_ | — | — | — | — |\n");
    }

    if rows.iter().any(|r| !r.secondary_metrics.is_empty()) {
        out.push_str("\n### Secondary metrics\n\n");
        for r in rows {
            if r.secondary_metrics.is_empty() {
                continue;
            }
            out.push_str(&format!("**{}**:\n", r.retriever));
            for (k, v) in &r.secondary_metrics {
                out.push_str(&format!("- `{k}` = {v:.3}\n"));
            }
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "**SPEC GATE: {pass} pass, {fail} fail (target: all pass)**\n"
    ));
    out
}
