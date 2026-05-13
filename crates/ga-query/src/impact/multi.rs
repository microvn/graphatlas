//! Cluster C8 — AS-013 multi-file union. Takes `changed_files`, extracts
//! every symbol defined in each file from the graph, unions the per-symbol
//! impacts, and folds the results back into a single [`ImpactResponse`].

use super::types::{
    AffectedConfig, AffectedRoute, AffectedTest, BreakPoint, ImpactRequest, ImpactResponse,
    ImpactedFile,
};
use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use std::collections::HashMap;

pub(super) fn fill_from_changed_files(
    store: &Store,
    files: &[String],
    max_depth: u32,
    resp: &mut ImpactResponse,
    req: &ImpactRequest,
) -> Result<()> {
    // Tools-C9-d — reject per-path that would break Cypher string literals;
    // empty / whitespace-only paths are meaningless input.
    let safe_files: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|f| {
            !f.trim().is_empty() && !f.contains('\'') && !f.contains('\n') && !f.contains('\r')
        })
        .collect();
    if safe_files.is_empty() {
        return Ok(());
    }

    let mut imp_map: HashMap<String, ImpactedFile> = HashMap::new();
    let mut tests_map: HashMap<String, AffectedTest> = HashMap::new();
    let mut routes_map: HashMap<(String, String, String), AffectedRoute> = HashMap::new();
    let mut configs_map: HashMap<(String, u32), AffectedConfig> = HashMap::new();
    let mut bp_map: HashMap<(String, u32), Vec<String>> = HashMap::new();
    let mut max_completeness: u32 = 0;

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;

    for file in safe_files {
        let symbols = query_symbols_in_file(&conn, file)?;
        for sym in symbols {
            if !common::is_safe_ident(&sym) {
                continue;
            }
            let mut partial = ImpactResponse {
                meta: super::types::ImpactMeta {
                    transitive_completeness: 0,
                    max_depth,
                    ..Default::default()
                },
                ..Default::default()
            };
            super::fill_from_symbol(store, &sym, max_depth, &mut partial, req)?;

            max_completeness = max_completeness.max(partial.meta.transitive_completeness);
            merge_impacted(&mut imp_map, partial.impacted_files);
            merge_tests(&mut tests_map, partial.affected_tests);
            merge_routes(&mut routes_map, partial.affected_routes);
            merge_configs(&mut configs_map, partial.affected_configs);
            merge_break_points(&mut bp_map, partial.break_points);
        }
    }

    resp.impacted_files = drain_impacted(imp_map);
    resp.affected_tests = drain_tests(tests_map);
    resp.affected_routes = drain_routes(routes_map);
    resp.affected_configs = drain_configs(configs_map);
    resp.break_points = drain_break_points(bp_map);
    resp.meta.transitive_completeness = max_completeness;
    Ok(())
}

fn query_symbols_in_file(conn: &lbug::Connection<'_>, file: &str) -> Result<Vec<String>> {
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.file = '{file}' AND s.kind <> 'external' \
         RETURN DISTINCT s.name"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("symbols-in-file query: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(name)) = row.into_iter().next() {
            out.push(name);
        }
    }
    Ok(out)
}

fn merge_impacted(map: &mut HashMap<String, ImpactedFile>, items: Vec<ImpactedFile>) {
    for item in items {
        match map.get(&item.path) {
            Some(existing) if existing.depth <= item.depth => {} // keep min-depth entry
            _ => {
                map.insert(item.path.clone(), item);
            }
        }
    }
}

fn merge_tests(map: &mut HashMap<String, AffectedTest>, items: Vec<AffectedTest>) {
    for item in items {
        map.entry(item.path.clone()).or_insert(item);
    }
}

fn merge_routes(
    map: &mut HashMap<(String, String, String), AffectedRoute>,
    items: Vec<AffectedRoute>,
) {
    for item in items {
        let key = (
            item.method.clone(),
            item.path.clone(),
            item.source_file.clone(),
        );
        map.entry(key).or_insert(item);
    }
}

fn merge_configs(map: &mut HashMap<(String, u32), AffectedConfig>, items: Vec<AffectedConfig>) {
    for item in items {
        map.entry((item.path.clone(), item.line)).or_insert(item);
    }
}

fn merge_break_points(map: &mut HashMap<(String, u32), Vec<String>>, items: Vec<BreakPoint>) {
    for item in items {
        map.entry((item.file.clone(), item.line))
            .or_default()
            .extend(item.caller_symbols);
    }
}

fn drain_impacted(map: HashMap<String, ImpactedFile>) -> Vec<ImpactedFile> {
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
    v
}

fn drain_tests(map: HashMap<String, AffectedTest>) -> Vec<AffectedTest> {
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

fn drain_routes(map: HashMap<(String, String, String), AffectedRoute>) -> Vec<AffectedRoute> {
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    v
}

fn drain_configs(map: HashMap<(String, u32), AffectedConfig>) -> Vec<AffectedConfig> {
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
    v
}

fn drain_break_points(map: HashMap<(String, u32), Vec<String>>) -> Vec<BreakPoint> {
    let mut v: Vec<BreakPoint> = map
        .into_iter()
        .map(|((file, line), mut syms)| {
            syms.sort();
            syms.dedup();
            BreakPoint {
                file,
                line,
                caller_symbols: syms,
            }
        })
        .collect();
    v.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    v
}
