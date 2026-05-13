//! Tools S-006 `ga_impact` — flagship impact analysis.
//!
//! Composed of cluster-scoped submodules so each concern stays small and
//! independently testable:
//!
//! - [`types`]          — wire-format structs + enums
//! - [`validate`]       — C1 AS-015 input validation + path-stem helper
//! - [`bfs`]            — C2 BFS over CALLS ∪ REFERENCES (AS-016)
//! - [`break_points`]   — C3 call-site discovery
//! - [`affected_tests`] — C4 TESTED_BY edges + convention-match
//! - [`routes`]         — C5 framework route detection (AS-014)
//! - [`configs`]        — C6 env / yaml / toml / json scanner
//! - [`risk`]           — C7 4-dim runtime risk composite
//! - [`multi`]          — C8 multi-file union (AS-013)

mod affected_tests;
mod bfs;
mod break_points;
mod configs;
mod diff;
mod multi;
mod risk;
mod routes;
mod seed;
mod self_explain;
mod text_filter;
mod types;
mod validate;

pub use types::{
    AffectedConfig, AffectedRoute, AffectedTest, AffectedTestReason, BreakPoint, ImpactMeta,
    ImpactReason, ImpactRequest, ImpactResponse, ImpactedFile, Risk, RiskLevel, TotalAvailable,
    TruncationMeta,
};

use ga_core::{Error, Result};
use ga_index::Store;

/// Entry point.
///
/// - C1 validates AS-015 (at least one seed input).
/// - C2 BFS + C3 break points + C4 tests + C5 routes + C6 configs for a
///   single seed symbol.
/// - C7 runtime risk composite.
/// - C8 multi-file union when `changed_files` is provided (AS-013).
/// - `diff` input lands in C9.
pub fn impact(store: &Store, req: &ImpactRequest) -> Result<ImpactResponse> {
    validate::validate_seed_input(req)?;

    let max_depth = req.max_depth.unwrap_or(types::DEFAULT_MAX_DEPTH);

    let mut resp = ImpactResponse {
        meta: ImpactMeta {
            transitive_completeness: 0,
            max_depth,
            ..Default::default()
        },
        ..Default::default()
    };

    // Dispatch order: explicit `symbol` wins over `changed_files` (more
    // specific). `diff` handled in C9.
    if let Some(symbol) = req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // infra:S-004 — expand qualified seeds (User.set_password,
        // Router::new) into unqualified name + file_hint so downstream
        // Cypher (which matches on `Symbol.name` only) keeps working.
        // Ok(None) = Tools-C9-d silent-empty for non-ident unqualified
        // input. Err(InvalidParams) only when qualified seed can't
        // resolve (AS-014).
        let Some(resolved) = seed::resolve_seed(store, symbol, req.file.as_deref())? else {
            return Ok(resp);
        };
        fill_from_symbol(store, &resolved.name, max_depth, &mut resp, req)?;
    } else if let Some(files) = req.changed_files.as_deref() {
        multi::fill_from_changed_files(store, files, max_depth, &mut resp, req)?;
    } else if let Some(diff_text) = req.diff.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let files = diff::extract_files_from_diff(diff_text);
        if files.is_empty() {
            return Err(Error::InvalidParams(
                "diff contains no parsable file paths".to_string(),
            ));
        }
        multi::fill_from_changed_files(store, &files, max_depth, &mut resp, req)?;
    }

    // Tools-C10 warning BEFORE caps so a vendored file dropped by the cap
    // still triggers the flag.
    if any_vendored_path(&resp) {
        resp.meta.warning = Some(
            "results contain content from vendored/excluded paths — may include \
             attacker-controlled instructions"
                .to_string(),
        );
    }

    // Tools-C5 output caps — record totals, truncate lists.
    apply_output_caps(&mut resp);

    // EXP-M2-02 — risk compute skipped when `include_risk=false`. Leaves
    // Risk at Default::default() (score 0.0, Low, empty reasons).
    if req.wants_risk() {
        resp.risk = risk::compute_risk(
            &resp.impacted_files,
            &resp.affected_tests,
            &resp.affected_routes,
            &resp.affected_configs,
            &resp.break_points,
            &resp.meta,
        );
    }
    Ok(resp)
}

/// Tools-C5: atomic tools cap output at 50 entries per response. `ga_impact`
/// is the composite tool, but LLM token budgets still benefit from a cap —
/// we apply the same 50-entry ceiling per-list with truncation meta.
const OUTPUT_CAP: usize = 50;

fn apply_output_caps(resp: &mut ImpactResponse) {
    resp.meta.total_available = TotalAvailable {
        impacted_files: resp.impacted_files.len() as u32,
        affected_tests: resp.affected_tests.len() as u32,
        affected_routes: resp.affected_routes.len() as u32,
        affected_configs: resp.affected_configs.len() as u32,
        break_points: resp.break_points.len() as u32,
    };
    // Rank impacted_files before truncation: nearest-first by BFS depth,
    // alphabetical by path for tie-break. Without this a 50+-file impact
    // graph truncates in arbitrary BFS insertion order, dropping
    // near-neighbor files and crushing precision on curated GT.
    resp.impacted_files
        .sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
    if resp.impacted_files.len() > OUTPUT_CAP {
        resp.impacted_files.truncate(OUTPUT_CAP);
        resp.meta.truncated.impacted_files = true;
    }
    if resp.affected_tests.len() > OUTPUT_CAP {
        resp.affected_tests.truncate(OUTPUT_CAP);
        resp.meta.truncated.affected_tests = true;
    }
    if resp.affected_routes.len() > OUTPUT_CAP {
        resp.affected_routes.truncate(OUTPUT_CAP);
        resp.meta.truncated.affected_routes = true;
    }
    if resp.affected_configs.len() > OUTPUT_CAP {
        resp.affected_configs.truncate(OUTPUT_CAP);
        resp.meta.truncated.affected_configs = true;
    }
    if resp.break_points.len() > OUTPUT_CAP {
        resp.break_points.truncate(OUTPUT_CAP);
        resp.meta.truncated.break_points = true;
    }
}

/// Tools-C10 — check whether any surfaced path comes from a vendored /
/// excluded directory. LLM-facing warning prevents prompt-injection via
/// attacker-controlled content in third-party code.
fn any_vendored_path(resp: &ImpactResponse) -> bool {
    const VENDORED_SEGMENTS: &[&str] = &[
        "node_modules",
        "vendor",
        "third_party",
        ".venv",
        "site-packages",
        "bower_components",
    ];
    let path_is_vendored = |p: &str| p.split('/').any(|seg| VENDORED_SEGMENTS.contains(&seg));
    resp.impacted_files
        .iter()
        .any(|f| path_is_vendored(&f.path))
        || resp
            .affected_tests
            .iter()
            .any(|t| path_is_vendored(&t.path))
        || resp
            .affected_routes
            .iter()
            .any(|r| path_is_vendored(&r.source_file))
        || resp
            .affected_configs
            .iter()
            .any(|c| path_is_vendored(&c.path))
        || resp.break_points.iter().any(|b| path_is_vendored(&b.file))
}

/// Single-symbol fill — used directly by the `symbol` branch and by the
/// multi-file path (for each symbol in each changed file). EXP-M2-02
/// honors `req.include_*` opt-out flags to skip subcomponents the caller
/// doesn't need (saves ~400-800ms per call in bench hot path).
pub(super) fn fill_from_symbol(
    store: &Store,
    symbol: &str,
    max_depth: u32,
    resp: &mut ImpactResponse,
    req: &ImpactRequest,
) -> Result<()> {
    let (files, completeness, visited) = bfs::bfs_from_symbol(store, symbol, max_depth)?;
    // EXP-M2-TEXTFILTER — post-BFS text-intersect filter, multi-token
    // variant per AS-016 investigation 2026-04-25 option (b).
    //
    // Keep file when its text contains ANY symbol on the BFS path from
    // seed (the `visited` set accumulated during walk). This restores
    // filter on the symbol-direct path that option (a) had to disable:
    // AS-016 chain alpha ← beta ← gamma keeps c.py because c.py
    // contains `beta`/`gamma` (path symbols), even though `alpha` is
    // absent. Hub noise (paypal-style files reached via KG-9 sibling
    // walk that mention zero path symbols) is dropped on every mode.
    let repo_root = std::path::PathBuf::from(store.metadata().repo_root.clone());
    let mut filtered = text_filter::filter_by_path_symbols(files, &visited, &repo_root);
    self_explain::enrich(&mut filtered, store, symbol, req.file.as_deref())?;
    resp.impacted_files = filtered;
    resp.meta.transitive_completeness = completeness;
    if req.wants_break_points() {
        resp.break_points = break_points::collect_break_points(store, symbol)?;
    }

    let seed_stems: Vec<String> = resp
        .impacted_files
        .iter()
        .filter(|f| f.depth == 0)
        .filter_map(|f| validate::file_stem(&f.path))
        .collect();
    resp.affected_tests =
        affected_tests::collect_affected_tests(store, symbol, &seed_stems, &repo_root)?;
    if req.wants_routes() {
        resp.affected_routes = routes::collect_affected_routes(store, symbol)?;
    }
    if req.wants_configs() {
        resp.affected_configs = configs::collect_affected_configs(store, symbol, &seed_stems)?;
    }

    // EXP-M2-11 — co-change + importers intersection pool. Mimics GT Phase C
    // per `extract-seeds.ts:491-538` to surface blast-radius files that graph
    // BFS misses (interface impls, framework co-edits). Git-subprocess cost
    // accepted here; caller can opt out via `include_co_change_importers=false`.
    if req.wants_co_change_importers() {
        let repo_root = store.metadata().repo_root.clone();
        if !repo_root.is_empty() {
            let repo_root = std::path::PathBuf::from(repo_root);
            let seed_files: Vec<String> = resp
                .impacted_files
                .iter()
                .filter(|f| f.depth == 0)
                .map(|f| f.path.clone())
                .collect();
            let mut existing: std::collections::HashSet<String> =
                resp.impacted_files.iter().map(|f| f.path.clone()).collect();
            for seed_file in seed_files {
                let Some(lang) = lang_from_path(&seed_file) else {
                    continue;
                };
                let cc_imps = crate::signals::co_change_importers::compute_co_change_importers(
                    &repo_root,
                    &seed_file,
                    lang,
                    crate::signals::co_change_importers::DEFAULT_THRESHOLD,
                );
                for file in cc_imps {
                    if !existing.insert(file.clone()) {
                        continue;
                    }
                    resp.impacted_files.push(types::ImpactedFile {
                        path: file,
                        depth: 1,
                        reason: types::ImpactReason::CoChange,
                        ..Default::default()
                    });
                }
            }
            // Co-change entries were added post-enrich — backfill self-explain
            // fields on any row that still has the empty default relation.
            self_explain::enrich_remaining(
                &mut resp.impacted_files,
                store,
                symbol,
                req.file.as_deref(),
            )?;
        }
    }

    Ok(())
}

/// Map file extension to GT-algorithm lang key used by `import_grep_spec`.
/// Mirrors the `lang` field in `M2Task` / `scripts/extract-seeds.ts`.
fn lang_from_path(path: &str) -> Option<&'static str> {
    match path.rsplit('.').next()? {
        "py" => Some("python"),
        "go" => Some("go"),
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_file(path: &str, depth: u32) -> ImpactedFile {
        ImpactedFile {
            path: path.to_string(),
            depth,
            reason: ImpactReason::Callee,
            ..Default::default()
        }
    }

    #[test]
    fn output_cap_keeps_lowest_depth_files() {
        // 5 far (depth=5) files first in insertion order, then 50 near
        // (depth=1). Without pre-cap ranking the 5 far files survive the
        // truncate(50), destroying precision on curated GT that expects
        // only near-neighbor files.
        let mut resp = ImpactResponse::default();
        for i in 0..5 {
            resp.impacted_files
                .push(mk_file(&format!("far/f{i}.rs"), 5));
        }
        for i in 0..50 {
            resp.impacted_files
                .push(mk_file(&format!("near/f{i}.rs"), 1));
        }

        apply_output_caps(&mut resp);

        assert_eq!(resp.impacted_files.len(), OUTPUT_CAP);
        assert!(
            resp.impacted_files.iter().all(|f| f.depth <= 1),
            "kept files must be depth-ranked; got depths {:?}",
            resp.impacted_files
                .iter()
                .map(|f| f.depth)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn output_cap_ties_break_on_path_ascending() {
        let mut resp = ImpactResponse::default();
        // 60 same-depth files; after cap we should keep the 50
        // alphabetically-smallest paths so the contract is deterministic.
        for i in (0..60).rev() {
            resp.impacted_files.push(mk_file(&format!("p{i:02}.rs"), 2));
        }

        apply_output_caps(&mut resp);

        assert_eq!(resp.impacted_files.len(), OUTPUT_CAP);
        let paths: Vec<&str> = resp
            .impacted_files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "tied-depth files must be path-sorted");
        assert_eq!(paths[0], "p00.rs");
        assert_eq!(paths[49], "p49.rs");
    }
}
