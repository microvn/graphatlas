//! Random baseline — seeded RNG picks N files from fixture. Null floor for
//! M2 gate: any retriever not meaningfully above random is exposed.

use crate::retriever::{ImpactActual, Retriever};
use crate::BenchError;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct RandomRetriever {
    seed: u64,
    pick: usize,
    files: Vec<String>,
    fixture_root: Option<PathBuf>,
}

impl RandomRetriever {
    pub fn new() -> Self {
        Self {
            // Pinned seed → deterministic across runs (anti-gaming)
            seed: 0xCAFE_BABE_DEAD_BEEF,
            pick: 20,
            files: Vec::new(),
            fixture_root: None,
        }
    }
}

impl Default for RandomRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl Retriever for RandomRetriever {
    fn name(&self) -> &str {
        "random"
    }

    fn setup(&mut self, fixture_dir: &Path) -> Result<(), BenchError> {
        self.fixture_root = Some(fixture_dir.to_path_buf());
        self.files = collect_source_files(fixture_dir);
        Ok(())
    }

    fn query(&mut self, _uc: &str, query: &Value) -> Result<Vec<String>, BenchError> {
        // Seed the RNG per-task by combining global seed with task-derived data
        // so "random" is deterministic & reproducible across re-runs, but
        // varies per task (can't cheat by always returning same files).
        let task_seed = task_hash(query).wrapping_add(self.seed);
        Ok(sample(&self.files, self.pick, task_seed))
    }

    fn query_impact(&mut self, query: &Value) -> Option<Result<ImpactActual, BenchError>> {
        let task_seed = task_hash(query).wrapping_add(self.seed);
        let picks = sample(&self.files, self.pick, task_seed);
        let tests: Vec<String> = picks.iter().filter(|p| is_test_path(p)).cloned().collect();
        let files: Vec<String> = picks.iter().filter(|p| !is_test_path(p)).cloned().collect();
        Some(Ok(ImpactActual {
            files,
            tests,
            routes: Vec::new(),
            transitive_completeness: 0,
            max_depth: 0,
        }))
    }
}

fn task_hash(query: &Value) -> u64 {
    // FxHash-style: fold query JSON bytes
    let s = serde_json::to_string(query).unwrap_or_default();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// Xorshift64 — small, deterministic, sufficient for sampling
fn next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    if x == 0 {
        x = 0xDEAD_BEEF;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn sample(files: &[String], n: usize, seed: u64) -> Vec<String> {
    if files.is_empty() {
        return Vec::new();
    }
    let n = n.min(files.len());
    // Fisher-Yates partial shuffle on indices
    let mut indices: Vec<usize> = (0..files.len()).collect();
    let mut state = seed;
    for i in 0..n {
        let j = (next_rand(&mut state) as usize) % (indices.len() - i) + i;
        indices.swap(i, j);
    }
    indices[..n].iter().map(|&i| files[i].clone()).collect()
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
        } else if is_interesting_source(&name_str) {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

fn is_interesting_source(name: &str) -> bool {
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
