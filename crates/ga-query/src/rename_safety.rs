//! S-004 ga_rename_safety — rename impact report.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-004):
//!   AS-011: Site enumeration — 1 definition + N call sites + M reference
//!     sites. Confidence: definition 1.0, CALLS 0.90, REFERENCES 0.70.
//!   AS-012: Blockers (string literals, external-package imports).
//!   AS-013: Polymorphic confidence — Tools-C11 file-hint narrowing.
//!
//! Read-only per Tools-C5: this tool returns a report; the agent decides
//! whether to proceed and invokes text-edit tools separately.
//!
//! Implementation notes:
//! - Definition sites read from `Symbol` nodes (kind <> 'external').
//! - Call sites read from CALLS edges; confidence composes AS-011 literal
//!   (0.90) with Tools-C11 polymorphic factor: 1.0 × 0.90 = 0.90 in the
//!   single-def OR file-hint-matches case; 0.6 (Tools-C11 floor) when
//!   ambiguous.
//! - Reference sites read from REFERENCES edges; same compose with 0.70.
//! - Column is computed at site-emit time by reading the source line and
//!   finding the first occurrence of the target token. Avoids a graph
//!   schema migration to add column data — site lines stored already.
//!   Column is 0-based byte offset into the line; falls back to 0 when
//!   the token is not found (e.g. dynamic dispatch where the line text
//!   doesn't literally contain the name).
//! - String-literal blockers scan source files for `"target"` / `'target'`
//!   occurrences. One blocker per file (collapsed). External-symbol
//!   blockers come from `Symbol{kind:'external', name=target}` rows.

use crate::common::{is_safe_ident, levenshtein};
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// AS-011 confidence literals.
const CONFIDENCE_DEFINITION: f32 = 1.0;
const CONFIDENCE_CALL: f32 = 0.90;
const CONFIDENCE_REFERENCE: f32 = 0.70;
/// AS-011 §Then: text-match (string-literal) sites get 0.50.
#[allow(dead_code)] // exposed via blocker reasons today; reserved for future text-site emission.
const CONFIDENCE_TEXT_MATCH: f32 = 0.50;
/// Tools-C11 polymorphic-ambiguous floor.
const CONFIDENCE_POLYMORPHIC: f32 = 0.6;
/// AS-004 / AS-016 cap on Levenshtein suggestions.
const SUGGESTION_LIMIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenameSiteKind {
    Definition,
    Call,
    Reference,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameSite {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub confidence: f32,
    pub kind: RenameSiteKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameBlocker {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenameSafetyReport {
    pub target: String,
    pub replacement: String,
    pub sites: Vec<RenameSite>,
    pub blocked: Vec<RenameBlocker>,
    /// PR14 / S-003 AS-009 (b) — DB-stored arity of the target symbol.
    /// `Some(-1)` is the Tools-C2 sentinel "non-function / unknown";
    /// callers should treat as not-comparable. Multi-def case picks the
    /// first match. `None` only when no arity column exists (pre-v4).
    pub existing_arity: Option<i64>,
    /// PR14 / S-003 AS-009 (b) — true when `RenameSafetyRequest.new_arity`
    /// is `Some(n)` AND `existing_arity == Some(m)` AND `m != n` AND
    /// `m != -1` (sentinel guard). False otherwise — including when no
    /// `new_arity` is provided, when target's arity is the unknown
    /// sentinel, and when the values match.
    pub param_count_changed: bool,
    /// v1.4 S-001c / AS-013 — count of subclass methods that override the
    /// target. Surfaces "renaming will silently break N subclass overrides"
    /// signal to the user. Computed via `MATCH (X)-[:OVERRIDES]->(target)`
    /// (single-step / immediate child overrides only — multi-level
    /// transitive descendants recovered via `OVERRIDES*` if a future story
    /// extends this; v1.4 ships single-step count).
    #[serde(default)]
    pub subclass_overrides_count: i64,
}

#[derive(Debug, Clone)]
pub struct RenameSafetyRequest {
    pub target: String,
    pub replacement: String,
    /// Optional Tools-C11 file hint — narrows the rename to symbols
    /// defined in the specified file when the target name is polymorphic.
    pub file_hint: Option<String>,
    /// PR14 / S-003 AS-009 (b) — proposed new arity for the target. When
    /// set, the report's `param_count_changed` flag indicates whether
    /// the rename also changes the parameter count (an API-breaking
    /// signal). `None` skips the check.
    pub new_arity: Option<i64>,
}

// ─────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────

pub fn rename_safety(store: &Store, req: &RenameSafetyRequest) -> Result<RenameSafetyReport> {
    validate(req)?;

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    if graph_is_empty(&conn)? {
        return Err(Error::IndexNotReady {
            status: "indexing".to_string(),
            progress: 0.0,
        });
    }

    let target = req.target.as_str();
    let file_hint = req.file_hint.as_deref().filter(|s| !s.is_empty());

    let def_sites_raw = collect_definition_rows(&conn, target)?;

    // Apply Tools-C11 file-hint narrowing on the def set.
    let def_files: Vec<String> = if let Some(hint) = file_hint {
        let hits: Vec<_> = def_sites_raw
            .iter()
            .filter(|d| d.file == hint)
            .map(|d| d.file.clone())
            .collect();
        if hits.is_empty() {
            // Hint didn't match any def — treat as if no hint was given (the
            // hint resolves to an empty set, which would otherwise return an
            // empty report; Tools-C11 semantic is "narrow when hit, otherwise
            // fall back to full set").
            def_sites_raw.iter().map(|d| d.file.clone()).collect()
        } else {
            hits
        }
    } else {
        def_sites_raw.iter().map(|d| d.file.clone()).collect()
    };
    let def_count = def_sites_raw.len() as i64;

    if def_count == 0 {
        // AS-011 / AS-016 — target not in graph → SymbolNotFound with
        // Levenshtein suggestions, structured per ga-mcp -32602 mapping.
        let suggestions = nearest_symbol_names(&conn, target)?;
        return Err(Error::SymbolNotFound { suggestions });
    }

    let repo_root = PathBuf::from(store.metadata().repo_root.clone());
    let mut sites: Vec<RenameSite> = Vec::new();

    // Definition sites — emit only those matching the narrowed def_files.
    for d in &def_sites_raw {
        if !def_files.contains(&d.file) {
            continue;
        }
        let column = find_token_column(&repo_root, &d.file, d.line, target);
        sites.push(RenameSite {
            file: d.file.clone(),
            line: d.line,
            column,
            confidence: CONFIDENCE_DEFINITION,
            kind: RenameSiteKind::Definition,
        });
    }

    // Call sites — emit per CALLS edge whose callee.name = target.
    for c in collect_call_rows(&conn, target)? {
        // Tools-C11: confidence drops to CONFIDENCE_POLYMORPHIC unless the
        // edge resolves to a single def OR (with file hint) the matching
        // def's file.
        let confidence = composed_confidence(
            CONFIDENCE_CALL,
            def_count,
            file_hint,
            &c.callee_file,
            &def_files,
        );
        // When a hint narrows the def set, drop sites whose callee.file is
        // outside the narrowed set entirely (AS-013 §Then literal:
        // "sites filtered to calls where polymorphic target resolves to
        // the specified class").
        if def_count > 1 && file_hint.is_some() && !def_files.contains(&c.callee_file) {
            continue;
        }
        let column = find_token_column(&repo_root, &c.caller_file, c.call_site_line, target);
        sites.push(RenameSite {
            file: c.caller_file,
            line: c.call_site_line,
            column,
            confidence,
            kind: RenameSiteKind::Call,
        });
    }

    // Reference sites.
    for r in collect_reference_rows(&conn, target)? {
        let confidence = composed_confidence(
            CONFIDENCE_REFERENCE,
            def_count,
            file_hint,
            &r.target_file,
            &def_files,
        );
        if def_count > 1 && file_hint.is_some() && !def_files.contains(&r.target_file) {
            continue;
        }
        let column = find_token_column(&repo_root, &r.caller_file, r.ref_site_line, target);
        sites.push(RenameSite {
            file: r.caller_file,
            line: r.ref_site_line,
            column,
            confidence,
            kind: RenameSiteKind::Reference,
        });
    }

    // Import sites — files that import the target name (best-effort).
    for path in collect_import_rows(&conn, target)? {
        sites.push(RenameSite {
            file: path,
            line: 0,
            column: 0,
            confidence: CONFIDENCE_CALL, // AS-011: import sites grouped with definitive code refs.
            kind: RenameSiteKind::Import,
        });
    }

    sites.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
    });

    let blocked = collect_blockers(&conn, &repo_root, target)?;

    // PR14 / S-003 AS-009 (b) — fetch DB arity for the target. Multi-def
    // case picks the first match in the (possibly file-hint-narrowed) set.
    // Tools-C2 sentinel: arity == -1 means "non-function / unknown" —
    // never raise param_count_changed against the sentinel.
    let existing_arity = fetch_target_arity(&conn, target, &def_files);
    let param_count_changed = match (req.new_arity, existing_arity) {
        (Some(new), Some(existing)) if existing != -1 && new != existing => true,
        _ => false,
    };

    // v1.4 S-001c / AS-013 — count subclass overrides for the target.
    let subclass_overrides_count = fetch_subclass_overrides_count(&conn, target, &def_files);

    Ok(RenameSafetyReport {
        target: req.target.clone(),
        replacement: req.replacement.clone(),
        sites,
        blocked,
        existing_arity,
        param_count_changed,
        subclass_overrides_count,
    })
}

/// v1.4 S-001c / AS-013 — count of subclass methods overriding the target.
/// Single-step (immediate `(child)-[:OVERRIDES]->(target)`); multi-level
/// transitive descendants would require `OVERRIDES*` Kleene support and
/// is deferred to a future story per Tools-C19a single-step emission. When
/// the target spans multiple defining files, sum across all matches.
fn fetch_subclass_overrides_count(
    conn: &lbug::Connection,
    target: &str,
    def_files: &[String],
) -> i64 {
    let safe_target = target.replace('\'', "");
    // Restrict to in-repo (file-narrowed) targets. If def_files is empty,
    // count any matching target name (rare — caller already raised
    // SymbolNotFound when unmatched).
    let q = if def_files.is_empty() {
        format!(
            "MATCH (child:Symbol)-[:OVERRIDES]->(target:Symbol {{name: '{safe_target}'}}) \
             WHERE target.kind <> 'external' \
             RETURN count(child)"
        )
    } else {
        // Pin to first def_file for determinism (multi-def is rare; single
        // count avoids double-counting across symbol-id collisions).
        let safe_file = def_files[0].replace('\'', "");
        format!(
            "MATCH (child:Symbol)-[:OVERRIDES]->(target:Symbol {{name: '{safe_target}', file: '{safe_file}'}}) \
             WHERE target.kind <> 'external' \
             RETURN count(child)"
        )
    };
    match conn.query(&q) {
        Ok(rs) => {
            for row in rs {
                if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
                    return n;
                }
            }
            0
        }
        Err(_) => 0,
    }
}

/// PR14 — fetch the target symbol's arity from DB. Returns `Some(-1)`
/// (Tools-C2 sentinel) for non-function symbols. Returns `None` when no
/// matching Symbol row exists (caller already raised SymbolNotFound).
fn fetch_target_arity(conn: &lbug::Connection, target: &str, def_files: &[String]) -> Option<i64> {
    // Quote-escape to keep the literal name CSV-safe inside Cypher.
    let safe_target = target.replace('\'', "");
    let q = format!(
        "MATCH (s:Symbol {{name: '{safe_target}'}}) WHERE s.kind <> 'external' \
         RETURN s.file, s.arity"
    );
    let rs = conn.query(&q).ok()?;
    let mut first_match: Option<i64> = None;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        let file = match cols.first() {
            Some(lbug::Value::String(s)) => s.clone(),
            _ => continue,
        };
        let arity = match cols.get(1) {
            Some(lbug::Value::Int64(n)) => *n,
            _ => continue,
        };
        // If file_hint narrowed def_files, prefer rows in that set.
        if def_files.iter().any(|f| f == &file) {
            return Some(arity);
        }
        if first_match.is_none() {
            first_match = Some(arity);
        }
    }
    first_match
}

// ─────────────────────────────────────────────────────────────────────────
// Validation
// ─────────────────────────────────────────────────────────────────────────

fn validate(req: &RenameSafetyRequest) -> Result<()> {
    if req.target.trim().is_empty() {
        return Err(Error::InvalidParams(
            "ga_rename_safety: `target` must be a non-empty identifier".to_string(),
        ));
    }
    if req.replacement.trim().is_empty() {
        return Err(Error::InvalidParams(
            "ga_rename_safety: `replacement` must be a non-empty identifier".to_string(),
        ));
    }
    if !is_safe_ident(&req.target) {
        return Err(Error::InvalidParams(
            "ga_rename_safety: `target` must match identifier charset (alnum / `_` / `$` / `.`)"
                .to_string(),
        ));
    }
    if !is_safe_ident(&req.replacement) {
        return Err(Error::InvalidParams(
            "ga_rename_safety: `replacement` must match identifier charset".to_string(),
        ));
    }
    if req.target == req.replacement {
        return Err(Error::InvalidParams(
            "ga_rename_safety: `target` and `replacement` must differ".to_string(),
        ));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Confidence composition (Tools-C11)
// ─────────────────────────────────────────────────────────────────────────

fn composed_confidence(
    base: f32,
    def_count: i64,
    file_hint: Option<&str>,
    callee_file: &str,
    def_files: &[String],
) -> f32 {
    if def_count <= 1 {
        return base;
    }
    if let Some(_hint) = file_hint {
        if def_files.iter().any(|f| f == callee_file) {
            return base;
        }
    }
    CONFIDENCE_POLYMORPHIC
}

// ─────────────────────────────────────────────────────────────────────────
// Graph queries
// ─────────────────────────────────────────────────────────────────────────

fn graph_is_empty(conn: &lbug::Connection<'_>) -> Result<bool> {
    let rs = conn
        .query("MATCH (s:Symbol) RETURN count(s)")
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety count: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n == 0);
        }
    }
    Ok(true)
}

struct DefRow {
    file: String,
    line: u32,
}

fn collect_definition_rows(conn: &lbug::Connection<'_>, target: &str) -> Result<Vec<DefRow>> {
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{}' AND s.kind <> 'external' \
         RETURN s.file, s.line",
        target
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety defs: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 2 {
            continue;
        }
        let file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        out.push(DefRow { file, line });
    }
    Ok(out)
}

struct CallRow {
    caller_file: String,
    call_site_line: u32,
    callee_file: String,
}

fn collect_call_rows(conn: &lbug::Connection<'_>, target: &str) -> Result<Vec<CallRow>> {
    let cypher = format!(
        "MATCH (caller:Symbol)-[r:CALLS]->(callee:Symbol) \
         WHERE callee.name = '{}' \
         RETURN caller.file, r.call_site_line, callee.file",
        target
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety calls: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 3 {
            continue;
        }
        let caller_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let call_site_line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let callee_file = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        out.push(CallRow {
            caller_file,
            call_site_line,
            callee_file,
        });
    }
    Ok(out)
}

struct RefRow {
    caller_file: String,
    ref_site_line: u32,
    target_file: String,
}

fn collect_reference_rows(conn: &lbug::Connection<'_>, target: &str) -> Result<Vec<RefRow>> {
    let cypher = format!(
        "MATCH (caller:Symbol)-[r:REFERENCES]->(target:Symbol) \
         WHERE target.name = '{}' \
         RETURN caller.file, r.ref_site_line, target.file",
        target
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety refs: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 3 {
            continue;
        }
        let caller_file = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let ref_site_line = match &cols[1] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let target_file = match &cols[2] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        out.push(RefRow {
            caller_file,
            ref_site_line,
            target_file,
        });
    }
    Ok(out)
}

fn collect_import_rows(conn: &lbug::Connection<'_>, target: &str) -> Result<Vec<String>> {
    // imported_names is a CSV-encoded array per indexer's bulk-load format —
    // we test contains-substring with simple word-boundary check.
    let cypher = "MATCH (src:File)-[r:IMPORTS]->(dst:File) \
                  RETURN src.path, r.imported_names";
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety imports: {e}")))?;
    let mut out: Vec<String> = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 2 {
            continue;
        }
        let path = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let names = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        if imported_names_contains(&names, target) {
            out.push(path);
        }
    }
    Ok(out)
}

fn imported_names_contains(csv: &str, target: &str) -> bool {
    csv.split(',').any(|name| name.trim() == target)
}

// ─────────────────────────────────────────────────────────────────────────
// Blockers (AS-012)
// ─────────────────────────────────────────────────────────────────────────

fn collect_blockers(
    conn: &lbug::Connection<'_>,
    repo_root: &std::path::Path,
    target: &str,
) -> Result<Vec<RenameBlocker>> {
    let mut by_file: BTreeMap<String, String> = BTreeMap::new();

    // 1) String-literal blockers — scan source files for `"target"` /
    //    `'target'` and record any file with at least 1 hit.
    let files = list_source_files(conn)?;
    for path in files {
        let abs = repo_root.join(&path);
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        if file_has_string_literal(text, target) {
            by_file.insert(path.clone(), format!("string literal contains `{target}`"));
        }
    }

    // 2) External-symbol blockers — `Symbol{kind:'external', name=target}`.
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{}' AND s.kind = 'external' \
         RETURN s.file",
        target
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety externals: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::String(file)) = row.into_iter().next() {
            by_file
                .entry(file)
                .and_modify(|reason| {
                    if !reason.contains("external") {
                        reason.push_str("; external package symbol");
                    }
                })
                .or_insert_with(|| format!("external package symbol `{target}`"));
        }
    }

    Ok(by_file
        .into_iter()
        .map(|(file, reason)| RenameBlocker { file, reason })
        .collect())
}

fn list_source_files(conn: &lbug::Connection<'_>) -> Result<Vec<String>> {
    let rs = conn
        .query("MATCH (f:File) RETURN f.path")
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety files: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(p)) = row.into_iter().next() {
            out.push(p);
        }
    }
    Ok(out)
}

/// Returns `true` when `text` contains the target name inside a string
/// literal (single- or double-quoted, line-local). Conservative — we only
/// look on lines that contain a quote character to avoid matching the
/// symbol's own declaration.
fn file_has_string_literal(text: &str, target: &str) -> bool {
    for line in text.lines() {
        if !line.contains('"') && !line.contains('\'') {
            continue;
        }
        // Walk characters tracking simple in-string state. Single-line only.
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'"' || c == b'\'' {
                let quote = c;
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != quote {
                    if bytes[j] == b'\\' && j + 1 < bytes.len() {
                        j += 2;
                        continue;
                    }
                    j += 1;
                }
                if j > start {
                    let body = &line[start..j.min(line.len())];
                    if literal_contains_word(body, target) {
                        return true;
                    }
                }
                i = j + 1;
                continue;
            }
            i += 1;
        }
    }
    false
}

/// Word-boundary substring match — `target` must appear with ASCII-non-ident
/// flanks (or string boundaries) so `User` doesn't match inside `Username`.
fn literal_contains_word(haystack: &str, target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let target_bytes = target.as_bytes();
    let n = bytes.len();
    let m = target_bytes.len();
    if m == 0 || n < m {
        return false;
    }
    let mut i = 0;
    while i + m <= n {
        if &bytes[i..i + m] == target_bytes {
            let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let next_ok = i + m == n || !is_ident_byte(bytes[i + m]);
            if prev_ok && next_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

// ─────────────────────────────────────────────────────────────────────────
// Column resolution — find first occurrence of `target` on the source line.
// ─────────────────────────────────────────────────────────────────────────

fn find_token_column(repo_root: &std::path::Path, file: &str, line: u32, target: &str) -> u32 {
    if line == 0 {
        return 0;
    }
    let abs = repo_root.join(file);
    let Ok(bytes) = std::fs::read(&abs) else {
        return 0;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return 0;
    };
    let Some(line_text) = text.lines().nth((line - 1) as usize) else {
        return 0;
    };
    let line_bytes = line_text.as_bytes();
    let target_bytes = target.as_bytes();
    let n = line_bytes.len();
    let m = target_bytes.len();
    if m == 0 || n < m {
        return 0;
    }
    let mut i = 0;
    while i + m <= n {
        if &line_bytes[i..i + m] == target_bytes {
            let prev_ok = i == 0 || !is_ident_byte(line_bytes[i - 1]);
            let next_ok = i + m == n || !is_ident_byte(line_bytes[i + m]);
            if prev_ok && next_ok {
                return i as u32;
            }
        }
        i += 1;
    }
    0
}

// ─────────────────────────────────────────────────────────────────────────
// Levenshtein suggestions for unknown target.
// ─────────────────────────────────────────────────────────────────────────

fn nearest_symbol_names(conn: &lbug::Connection<'_>, target: &str) -> Result<Vec<String>> {
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN DISTINCT s.name")
        .map_err(|e| Error::Other(anyhow::anyhow!("rename_safety nearest: {e}")))?;
    let mut scored: Vec<(u32, String)> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(name)) = row.into_iter().next() {
            let d = levenshtein(target, &name);
            scored.push((d, name));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(scored
        .into_iter()
        .take(SUGGESTION_LIMIT)
        .map(|(_, n)| n)
        .collect())
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn literal_contains_word_word_boundary() {
        assert!(literal_contains_word("Hello User!", "User"));
        assert!(!literal_contains_word("Username", "User"));
    }

    #[test]
    fn imported_names_csv_match_is_word_exact() {
        assert!(imported_names_contains("foo,bar,baz", "bar"));
        assert!(!imported_names_contains("foobar", "bar"));
    }

    #[test]
    fn composed_confidence_single_def_uses_base() {
        let c = composed_confidence(0.90, 1, None, "a.py", &["a.py".into()]);
        assert!((c - 0.90).abs() < 1e-6);
    }

    #[test]
    fn composed_confidence_polymorphic_no_hint_drops_to_floor() {
        let c = composed_confidence(0.90, 3, None, "a.py", &["a.py".into(), "b.py".into()]);
        assert!((c - CONFIDENCE_POLYMORPHIC).abs() < 1e-6);
    }

    #[test]
    fn composed_confidence_polymorphic_with_matching_hint_uses_base() {
        let c = composed_confidence(0.90, 3, Some("a.py"), "a.py", &["a.py".into()]);
        assert!((c - 0.90).abs() < 1e-6);
    }

    #[test]
    fn file_has_string_literal_double_and_single_quote() {
        assert!(file_has_string_literal("X = \"hello User\"\n", "User"));
        assert!(file_has_string_literal("Y = 'User logged in'\n", "User"));
    }

    #[test]
    fn file_has_string_literal_skips_code_form() {
        assert!(!file_has_string_literal("def User():\n    pass\n", "User"));
    }
}
