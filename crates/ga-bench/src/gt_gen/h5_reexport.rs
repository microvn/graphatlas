//! H5 — TS / JS re-export chain. For each file that has ≥2 importers
//! reaching it through a `re_export: true` hop (depth ≤ 3 per Tools-C12),
//! emit an `importers` task whose expected list is the set of those
//! transitive importers.
//!
//! Why this is a "hard" case: a lexical tool (ripgrep, grep) grepping for
//! the leaf file's name misses the chain because intermediate re-export
//! files don't mention the leaf's path — only `export * from './hop'`.
//! Graph retrievers with chain-aware import resolution catch them.
//!
//! The expected set is whatever `ga_query::importers` returns — which is
//! the structural union of direct + depth-2 + depth-3 importers. That is
//! the documented rule; tools that implement only direct-importer
//! resolution score lower, and that's what the benchmark wants to show.

use super::{GeneratedTask, GtRule};
use crate::BenchError;
use ga_core::Error as GaError;
use ga_index::Store;
use serde_json::json;
use std::path::Path;

pub struct H5ReExport;

impl GtRule for H5ReExport {
    fn id(&self) -> &str {
        "H5-reexport"
    }
    fn uc(&self) -> &str {
        "importers"
    }
    fn scan(&self, store: &Store, _fixture_dir: &Path) -> Result<Vec<GeneratedTask>, BenchError> {
        let conn = store
            .connection()
            .map_err(|e| BenchError::Other(anyhow::anyhow!("connection: {e}")))?;

        // Find every File that some other file imports. We don't filter on
        // language here — TS/JS dominate re-export chains, but Python
        // package-level __init__.py can do similar via `from .x import *`.
        // The importers() query downstream will surface only real chains.
        let rs = conn
            .query("MATCH (f:File) RETURN f.path")
            .map_err(|e| BenchError::Other(anyhow::anyhow!("file enum: {e}")))?;
        let mut files: Vec<String> = Vec::new();
        for row in rs {
            if let Some(lbug::Value::String(p)) = row.into_iter().next() {
                files.push(p);
            }
        }

        let mut out = Vec::new();
        for file in files {
            let resp = match ga_query::importers(store, &file) {
                Ok(r) => r,
                Err(GaError::Other(_)) | Err(_) => continue,
            };
            // Only emit if at least one transitive (re_export) importer exists —
            // otherwise it's a direct-importer case that's already trivial GT.
            let has_chain = resp.importers.iter().any(|e| e.re_export);
            if !has_chain || resp.importers.len() < 2 {
                continue;
            }
            let expected: Vec<String> = resp.importers.iter().map(|e| e.path.clone()).collect();
            out.push(GeneratedTask {
                task_id: format!("reexport_{}", sanitize(&file)),
                query: json!({ "file": file }),
                expected,
                rule: self.id().to_string(),
                rationale: "file has ≥1 re_export=true chain importer + ≥2 total — lexical baseline misses the chain".to_string(),
            });
        }
        Ok(out)
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
