//! v1.5 PR9 triggers S-004 — incremental reindex pipeline.
//!
//! When a watcher (Layer 1) or hook (Layer 2) fires, this module
//! computes the smallest re-parse set needed to bring the graph back in
//! sync with the working tree:
//!
//! 1. **Dirty paths** — walk the repo (via `ga_parser::walk_repo` so
//!    `.gitignore` is respected) and compare each file's BLAKE3 to the
//!    sha256 column on its File row. Mismatch + missing-on-disk
//!    entries surface as dirty. Spec parity with AS-012 ("gix diff
//!    HEAD → working tree"): the gix-based implementation was carved
//!    out per the AS-012 spike-deferral clause (see `Cargo.toml` for
//!    PR9.1 destination); this sha256-vs-disk fallback delivers the
//!    same functional contract without the gix dep.
//! 2. **SHA-256 filter** — collapsed into step 1 in this MVP, since
//!    the snapshot read happens BEFORE any DELETE. AS-013 invariant
//!    holds: snapshot → BLAKE3 → filter, no read-after-DELETE window.
//! 3. **Dependent expansion** — 2-hop reverse-edge BFS via Cypher on
//!    `IMPORTS|CALLS|IMPORTS_NAMED|CALLS_HEURISTIC` (AS-014). Capped
//!    at 500 files; truncated flag mirrors CRG.
//! 4. **Compile-time gate** — `INCREMENTAL_ENABLED` toggles between the
//!    incremental path and the safe full-rebuild fallback (AS-015/016).
//!    Default `false` so a fresh clone is safe-by-default: full rebuild
//!    every time until the Phase F artifact wires the flag on.

use anyhow::{anyhow, Result};
use ga_index::Store;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// v1.5 PR9 AS-015 / AS-016 — compile-time gate. Flipped to `true` once
/// the Phase F empirical artifact for `cross_file_edge_integrity` PASSes
/// on the target platform (see `scripts/run-lbug-empirical.sh` +
/// `.github/workflows/ci.yml::lbug-empirical`). Default `false` so a
/// fresh clone or a build that hasn't yet seen the artifact takes the
/// safe full-rebuild path.
///
/// Wire-up plan (deferred → PR9.1): a `build.rs` reads
/// `target/lbug_lifecycle/as_001_*.json` + `as_002_*.json`, emits
/// `cargo:rustc-env=INCREMENTAL_ENABLED=true` only when both report
/// `result: "PASS"`. Until then this constant stays `false` and the
/// pipeline always falls back to full rebuild.
pub const INCREMENTAL_ENABLED: bool = false;

/// AS-014 — hard cap on the re-parse set size. Mirrors CRG. If the
/// 2-hop dependent expansion exceeds this, the result is marked
/// `truncated = true` and the pipeline caller falls back to full
/// rebuild rather than ship a partial.
pub const MAX_REPARSE_FILES: usize = 500;

/// Result of one incremental pipeline planning pass. Caller decides
/// whether to execute (DELETE+INSERT cycle inside lbug transaction) or
/// fall back to full rebuild based on `truncated` + `INCREMENTAL_ENABLED`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncrementalPlan {
    /// Paths whose disk content differs from the indexed sha256 (or
    /// paths newly appeared on disk vs the graph).
    pub changed: Vec<PathBuf>,
    /// Paths added by the 2-hop reverse-edge dependent expansion.
    /// Disjoint from `changed`.
    pub dependents: Vec<PathBuf>,
    /// True if `changed ∪ dependents` would exceed `MAX_REPARSE_FILES`
    /// before truncation. Caller MUST fall back to full rebuild when
    /// this is set.
    pub truncated: bool,
}

impl IncrementalPlan {
    /// Total re-parse set (changed ∪ dependents). Order: changed paths
    /// first (sorted), then dependents (sorted). Stable across runs.
    pub fn reparse_set(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self.changed.clone();
        out.extend(self.dependents.iter().cloned());
        out
    }
}

// =====================================================================
// AS-012 + AS-013 — dirty path detection via sha256-vs-disk comparison
// =====================================================================

/// AS-012 + AS-013 — return the list of files whose disk content
/// differs from what the graph last indexed. Combines the dirty-path
/// detection and the SHA-256 skip filter into one snapshot pass:
/// read all File.sha256 from the graph BEFORE comparing against disk,
/// so the AS-013 invariant ("snapshot before DELETE") holds.
///
/// Paths are returned canonical-relative to `repo_root` with
/// forward-slash separators (Windows-safe). Files missing on disk
/// (deletions) ARE included so the caller can DELETE their rows.
pub fn dirty_paths(store: &Store, repo_root: &Path) -> Result<Vec<PathBuf>> {
    let snapshot = snapshot_all_indexed_sha256(store)?;
    let mut dirty: HashSet<PathBuf> = HashSet::new();

    // Walk current disk state. Anything walk_repo finds is in the
    // current source set — diff against snapshot to find modified +
    // newly-added files.
    let report = ga_parser::walk::walk_repo(repo_root)
        .map_err(|e| anyhow!("dirty_paths: walk_repo failed: {e}"))?;
    let mut on_disk: HashSet<PathBuf> = HashSet::new();
    for entry in &report.entries {
        let rel = normalize_relpath(&entry.rel_path);
        on_disk.insert(rel.clone());
        let bytes = match std::fs::read(&entry.abs_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let h: [u8; 32] = *blake3::hash(&bytes).as_bytes();
        let same = snapshot.get(&rel).map(|s| s == &h).unwrap_or(false);
        if !same {
            dirty.insert(rel);
        }
    }

    // Indexed files that no longer exist on disk → DELETE pending.
    for indexed_path in snapshot.keys() {
        if !on_disk.contains(indexed_path) {
            dirty.insert(indexed_path.clone());
        }
    }

    let mut out: Vec<PathBuf> = dirty.into_iter().collect();
    out.sort();
    Ok(out)
}

/// AS-013 — read every `File.sha256` from the graph into an in-memory
/// snapshot. Done BEFORE any DELETE on the File table so the snapshot
/// is taken from intact data (H-5 fix). Empty graph (fresh build) →
/// empty map, which means every walked file looks "dirty" and the
/// incremental gate falls back to full rebuild on first run.
pub fn snapshot_all_indexed_sha256(store: &Store) -> Result<HashMap<PathBuf, [u8; 32]>> {
    let mut map = HashMap::new();
    let conn = store
        .connection()
        .map_err(|e| anyhow!("snapshot_all_indexed_sha256: open conn: {e}"))?;
    let cypher = "MATCH (f:File) RETURN f.path, f.sha256";
    let rs = match conn.query(cypher) {
        Ok(r) => r,
        Err(_) => return Ok(map), // schema not yet populated
    };
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 2 {
            continue;
        }
        let path = match &cols[0] {
            lbug::Value::String(s) if !s.is_empty() => PathBuf::from(s),
            _ => continue,
        };
        // `File.sha256` is stored as a BLOB (32 raw bytes) — see
        // ga-query/src/indexer.rs:963 (COPY File ... sha256 BLOB).
        // Some older callers may have stored it as a hex string; accept
        // both shapes for forward-compat with lbug schema migrations.
        let hash = match &cols[1] {
            lbug::Value::Blob(b) if b.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(b);
                arr
            }
            lbug::Value::String(s) => decode_hex32(s).unwrap_or([0u8; 32]),
            _ => [0u8; 32],
        };
        map.insert(normalize_relpath(&path), hash);
    }
    Ok(map)
}

fn normalize_relpath(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().replace('\\', "/");
    PathBuf::from(s)
}

// =====================================================================
// AS-014 — dependent expansion via 2-hop reverse-edge BFS
// =====================================================================

/// AS-014 — return the set of files that import or call into any of
/// `seed_files`, up to depth 2. Excludes the seeds themselves. Order:
/// sorted. Returns the dependents list AND a `truncated` flag set when
/// the combined `seed ∪ dependents` would exceed `MAX_REPARSE_FILES`.
///
/// Cypher walks the typed edges (`IMPORTS`, `CALLS`, `IMPORTS_NAMED`,
/// `CALLS_HEURISTIC`) that exist in schema v5. Path values are
/// quote-rejected (Tools-C9-d) so the inline format-string is safe.
pub fn expand_dependents(store: &Store, seed_files: &[PathBuf]) -> Result<(Vec<PathBuf>, bool)> {
    if seed_files.is_empty() {
        return Ok((Vec::new(), false));
    }
    let conn = store
        .connection()
        .map_err(|e| anyhow!("expand_dependents: open conn: {e}"))?;
    let mut deps: HashSet<PathBuf> = HashSet::new();
    let seed_set: HashSet<PathBuf> = seed_files.iter().cloned().collect();

    for seed in seed_files {
        let s = seed.to_string_lossy().to_string();
        // Tools-C9-d — reject paths with quote / newline so inline
        // format-string can't be hijacked.
        if s.contains('\'') || s.contains('\n') || s.contains('\r') {
            continue;
        }
        // 2-hop traversal: any File n that contains a Symbol referencing
        // a Symbol defined in seed via the typed reverse edges.
        let cypher = format!(
            "MATCH (n:File)-[:CONTAINS]->(:Symbol)-[:CALLS|IMPORTS|IMPORTS_NAMED|CALLS_HEURISTIC]->(t:Symbol) \
             WHERE t.file_path = '{}' \
             RETURN DISTINCT n.path LIMIT 1000",
            s
        );
        let rs = match conn.query(&cypher) {
            Ok(r) => r,
            Err(_) => continue, // best-effort; missing schema → no deps
        };
        for row in rs {
            let cols: Vec<lbug::Value> = row.into_iter().collect();
            if let Some(lbug::Value::String(p)) = cols.first() {
                if p.is_empty() {
                    continue;
                }
                let pb = normalize_relpath(&PathBuf::from(p));
                if !seed_set.contains(&pb) {
                    deps.insert(pb);
                }
            }
        }
    }
    let total = seed_files.len() + deps.len();
    let truncated = total > MAX_REPARSE_FILES;
    let mut out: Vec<PathBuf> = deps.into_iter().collect();
    out.sort();
    if truncated {
        out.truncate(MAX_REPARSE_FILES.saturating_sub(seed_files.len()));
    }
    Ok((out, truncated))
}

// =====================================================================
// AS-015 + AS-016 — top-level planner
// =====================================================================

/// Top-level pipeline planner. Returns `None` if `INCREMENTAL_ENABLED`
/// is false (compile-time fallback to full rebuild — AS-016), or if
/// the dirty set is large enough that the 2-hop expansion truncates.
/// `Some(plan)` means the caller may proceed with the per-file
/// DELETE+INSERT cycle. The caller still owns the actual lbug
/// transaction execution — this module only computes the *plan*.
pub fn plan(store: &Store, repo_root: &Path) -> Result<Option<IncrementalPlan>> {
    if !INCREMENTAL_ENABLED {
        tracing::info!(
            "incremental: INCREMENTAL_ENABLED=false; pipeline disabled, caller \
             should fall back to ga_reindex full rebuild"
        );
        return Ok(None);
    }
    let changed = dirty_paths(store, repo_root)?;
    if changed.is_empty() {
        return Ok(Some(IncrementalPlan {
            changed: Vec::new(),
            dependents: Vec::new(),
            truncated: false,
        }));
    }
    let (dependents, truncated) = expand_dependents(store, &changed)?;
    if truncated {
        tracing::warn!(
            seed = changed.len(),
            "incremental: dependent expansion truncated at MAX_REPARSE_FILES; \
             caller should fall back to full rebuild"
        );
        return Ok(None);
    }
    Ok(Some(IncrementalPlan {
        changed,
        dependents,
        truncated: false,
    }))
}

/// Decode a 64-char lowercase hex string into 32 bytes. Returns
/// `None` for any non-hex input — caller treats as "no snapshot
/// baseline" which falls through to BLAKE3 mismatch.
fn decode_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        out[i] = byte;
    }
    Some(out)
}
