//! BM25 text-retrieval baseline — pure Rust, zero extra deps.
//!
//! Indexes every source file as a single document (token = identifier-like
//! word). Query = seed symbol + seed file basename. Returns top-N files
//! ranked by BM25 Okapi score.
//!
//! This is the IR-community floor: any graph retriever that doesn't
//! meaningfully beat BM25 hasn't demonstrated value from structure.

use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const K1: f64 = 1.5;
const B: f64 = 0.75;
const TOP_N: usize = 50;

pub struct Bm25Retriever {
    docs: Vec<Document>,
    // term → list of (doc_idx, tf)
    postings: HashMap<String, Vec<(usize, u32)>>,
    avg_doc_len: f64,
    fixture_root: Option<PathBuf>,
}

struct Document {
    path: String,
    len: u32,
}

impl Bm25Retriever {
    pub fn new() -> Self {
        Self {
            docs: Vec::new(),
            postings: HashMap::new(),
            avg_doc_len: 0.0,
            fixture_root: None,
        }
    }

    fn search(&self, query_terms: &[String], top_n: usize) -> Vec<String> {
        if self.docs.is_empty() {
            return Vec::new();
        }
        let n = self.docs.len() as f64;
        let mut scores: Vec<f64> = vec![0.0; self.docs.len()];

        for term in query_terms {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            let df = postings.len() as f64;
            // BM25 IDF (Okapi plus variant with +0.5 smoothing)
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (doc_idx, tf) in postings {
                let tf = *tf as f64;
                let dl = self.docs[*doc_idx].len as f64;
                let norm = 1.0 - B + B * (dl / self.avg_doc_len.max(1.0));
                let score = idf * (tf * (K1 + 1.0)) / (tf + K1 * norm);
                scores[*doc_idx] += score;
            }
        }

        let mut ranked: Vec<(usize, f64)> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, s)| *s > 0.0)
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_n);
        ranked
            .into_iter()
            .map(|(i, _)| self.docs[i].path.clone())
            .collect()
    }
}

impl Default for Bm25Retriever {
    fn default() -> Self {
        Self::new()
    }
}

impl Retriever for Bm25Retriever {
    fn name(&self) -> &str {
        "bm25"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_root = Some(fixture_dir.to_path_buf());
        self.docs.clear();
        self.postings.clear();

        let files = collect_source_files(fixture_dir);
        let mut total_len: u64 = 0;
        for rel in files {
            let abs = fixture_dir.join(&rel);
            let Ok(content) = std::fs::read_to_string(&abs) else {
                continue;
            };
            // Count tokens + build per-doc TF map
            let mut tf: HashMap<String, u32> = HashMap::new();
            let mut len = 0u32;
            for tok in tokenize(&content) {
                *tf.entry(tok).or_insert(0) += 1;
                len += 1;
            }
            if len == 0 {
                continue;
            }
            let doc_idx = self.docs.len();
            self.docs.push(Document { path: rel, len });
            total_len += len as u64;
            for (term, count) in tf {
                self.postings
                    .entry(term)
                    .or_default()
                    .push((doc_idx, count));
            }
        }
        if self.docs.is_empty() {
            self.avg_doc_len = 1.0;
        } else {
            self.avg_doc_len = total_len as f64 / self.docs.len() as f64;
        }
        Ok(())
    }

    fn query(&mut self, _uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        let terms = query_terms(query);
        Ok(self.search(&terms, TOP_N))
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        let terms = query_terms(query);
        let hits = self.search(&terms, TOP_N);
        let (files, tests): (Vec<_>, Vec<_>) = hits.into_iter().partition(|p| !is_test_path(p));
        Some(Ok(ImpactActual {
            files,
            tests,
            routes: Vec::new(),
            transitive_completeness: 0,
            max_depth: 0,
        }))
    }
}

fn query_terms(query: &Value) -> Vec<String> {
    let mut terms = Vec::new();
    if let Some(symbol) = query.get("symbol").and_then(|v| v.as_str()) {
        terms.push(symbol.to_lowercase());
        // snake_case → split
        for seg in symbol.split(|c: char| c == '_' || !c.is_alphanumeric()) {
            if seg.len() >= 2 {
                terms.push(seg.to_lowercase());
            }
        }
    }
    if let Some(file) = query.get("file").and_then(|v| v.as_str()) {
        let base = file.rsplit('/').next().unwrap_or(file);
        let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
        if stem.len() >= 2 {
            terms.push(stem.to_lowercase());
        }
    }
    terms
}

fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .flat_map(|word| {
            // Split snake_case + collect
            let mut parts = vec![word.to_lowercase()];
            for seg in word.split('_') {
                if seg.len() >= 2 {
                    parts.push(seg.to_lowercase());
                }
            }
            parts.into_iter()
        })
}

fn collect_source_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "node_modules"
                | "vendor"
                | "target"
                | "dist"
                | "build"
                | "__pycache__"
                | ".venv"
                | "venv"
        ) {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out);
        } else if is_interesting(&name_str) {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn is_interesting(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.rsplit_once('.').map(|(_, e)| e),
        Some("rs")
            | Some("go")
            | Some("py")
            | Some("ts")
            | Some("tsx")
            | Some("js")
            | Some("jsx")
            | Some("mjs")
            | Some("cjs")
    )
}

// S-002-bench §4.2.6 medium-term refactor — single canonical via
// `ga_query::common::is_test_path`.
use ga_query::common::is_test_path;
