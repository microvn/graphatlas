//! H1 — Polymorphism via inheritance. For each method `M` defined in a
//! class `A` that has ≥1 subclass overriding `M` (same name, different
//! enclosing class via the EXTENDS edge), emit a `callers(M, file=A_file)`
//! task. Expected list = union of caller names across all overriding
//! classes' methods — this is the "polymorphic blast radius" policy
//! (Tools-C11 pinned the same semantic on the query side).
//!
//! Why this is a "hard" case: a lexical tool grepping `M(` sees all sites
//! regardless of type; a same-name-in-different-file graph tool without
//! polymorphic expansion returns only direct callers of `A.M` and misses
//! callers that actually invoke the override. This rule encodes the
//! union-expansion policy as the ground truth.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_index::Store;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct H1Polymorphism;

impl GtRule for H1Polymorphism {
    fn id(&self) -> &str {
        "H1-polymorphism"
    }
    fn uc(&self) -> &str {
        "callers"
    }
    fn scan(&self, store: &Store, _fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let conn = store
            .connection()
            .map_err(|e| BenchError::Other(anyhow::anyhow!("connection: {e}")))?;

        // Step 1 — map class name → file where the class is defined. We only
        // consider classes; methods + functions are filtered downstream by
        // the EXTENDS traversal.
        // Cover Python/TS classes + Rust traits/structs/enums — anything a
        // child type can EXTEND. "class_files" keeps the same semantic
        // (parent type → its defining file).
        let rs = conn
            .query(
                "MATCH (s:Symbol) WHERE s.kind IN ['class', 'struct', 'trait', 'interface', 'enum'] \
                 RETURN s.name, s.file",
            )
            .map_err(|e| BenchError::Other(anyhow::anyhow!("classes: {e}")))?;
        let mut class_files: HashMap<String, String> = HashMap::new();
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            if let (lbug::Value::String(name), lbug::Value::String(file)) = (&cols[0], &cols[1]) {
                class_files.entry(name.clone()).or_insert(file.clone());
            }
        }

        // Step 2 — for each EXTENDS edge, record (child_class, parent_class).
        let rs = conn
            .query(
                "MATCH (child:Symbol)-[:EXTENDS]->(parent:Symbol) \
                 RETURN child.name, parent.name",
            )
            .map_err(|e| BenchError::Other(anyhow::anyhow!("extends: {e}")))?;
        let mut children_of: HashMap<String, Vec<String>> = HashMap::new();
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            if let (lbug::Value::String(child), lbug::Value::String(parent)) = (&cols[0], &cols[1])
            {
                children_of
                    .entry(parent.clone())
                    .or_default()
                    .push(child.clone());
            }
        }

        // Step 3 — for each parent class A with method M, check if any child
        // class B also defines M (override). Enumerate all methods-in-A.
        // Python tree-sitter calls `def` inside a class `function_definition`
        // so kind='function' — we accept both here. Class-file cross-reference
        // below filters to the ones that belong to a class anyway.
        let rs = conn
            .query(
                "MATCH (s:Symbol) WHERE s.kind = 'method' OR s.kind = 'function' \
                 RETURN s.name, s.file",
            )
            .map_err(|e| BenchError::Other(anyhow::anyhow!("methods: {e}")))?;
        // (file, method_name) — lets us check "does class A at file F have method M".
        let mut methods_in_file: HashMap<String, HashSet<String>> = HashMap::new();
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if cols.len() < 2 {
                continue;
            }
            if let (lbug::Value::String(name), lbug::Value::String(file)) = (&cols[0], &cols[1]) {
                methods_in_file
                    .entry(file.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }

        let mut out = Vec::new();
        let mut emitted_keys: HashSet<(String, String)> = HashSet::new();
        for (parent_class, parent_file) in &class_files {
            let Some(children) = children_of.get(parent_class) else {
                continue;
            };
            let Some(parent_methods) = methods_in_file.get(parent_file) else {
                continue;
            };
            for method in parent_methods {
                // Skip dunders (`__init__`, `__str__`, …) — hundreds of
                // callers across the repo, polymorphic expansion is
                // dominated by noise from every other class with the same
                // magic method. The bench wants DIFFERENTIATING cases.
                if is_dunder(method) {
                    continue;
                }
                // Skip 1-2 char names — too generic to bench meaningfully
                // (`f`, `cb`, `it`).
                if method.len() < 3 {
                    continue;
                }
                // Does any child class's file contain the same-named method?
                let mut override_files: Vec<String> = Vec::new();
                for child in children {
                    let Some(child_file) = class_files.get(child) else {
                        continue;
                    };
                    if let Some(child_methods) = methods_in_file.get(child_file) {
                        if child_methods.contains(method) {
                            override_files.push(child_file.clone());
                        }
                    }
                }
                if override_files.is_empty() {
                    continue;
                }
                // Dedupe by (method, parent_file) — multiple children override
                // same method → one task, not many.
                let key = (method.clone(), parent_file.clone());
                if !emitted_keys.insert(key) {
                    continue;
                }
                // Expected = union of callers across parent + every override
                // file. Use ga_query::callers which applies the Tools-C11
                // polymorphic-expansion rule — same policy as the bench GT.
                let mut expected: HashSet<String> = HashSet::new();
                let parent_callers = ga_query::callers(store, method, Some(parent_file))
                    .map_err(|e| BenchError::Query(e.to_string()))?;
                for c in parent_callers.callers {
                    expected.insert(c.symbol);
                }
                for of in &override_files {
                    let child_callers = ga_query::callers(store, method, Some(of))
                        .map_err(|e| BenchError::Query(e.to_string()))?;
                    for c in child_callers.callers {
                        expected.insert(c.symbol);
                    }
                }
                if expected.is_empty() {
                    continue; // no callers → not a useful bench task
                }
                // Cap expected set: methods shared by 30+ callers are too
                // generic (e.g. `save`, `handle`) — every test case calls
                // them, polymorphic expansion pulls in everything, F1
                // measurements become meaningless because expected is the
                // whole repo. A tighter "specificity signal" keeps the
                // bench discriminating.
                if expected.len() > 30 {
                    continue;
                }
                let mut expected_sorted: Vec<String> = expected.into_iter().collect();
                expected_sorted.sort();
                let task_id = format!("{}_{}", method, parent_class);
                out.push(GeneratedTask {
                    task_id,
                    query: json!({ "symbol": method, "file": parent_file }),
                    expected: expected_sorted,
                    rule: self.id().to_string(),
                    rationale: format!(
                        "{} defines {}; {} override(s) in subclass(es) — polymorphic expansion",
                        parent_class,
                        method,
                        override_files.len()
                    ),
                });
            }
        }
        Ok(out)
    }
}

fn is_dunder(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__") && name.len() >= 5
}
