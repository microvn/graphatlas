//! S-002 ga_minimal_context — token-budgeted context retriever.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-002):
//!   "Return the smallest context slice that captures the symbol's public
//!    API + direct callers/callees + type deps, so I can fit meaningful
//!    context in a constrained prompt."
//!
//! Priority order (AS-005 §Data) when budget binds:
//!   1. seed body
//!   2. callers signatures
//!   3. callees signatures
//!   4. imported types
//!   5. test examples (Phase D follow-up; not in v1)
//!
//! AS-007: budget too small → graceful partial with `meta.warning`,
//! never error. AS-016: symbol unknown → typed Err with Levenshtein
//! suggestions (`ga-mcp` maps to JSON-RPC -32602).

use crate::common::{is_safe_ident, levenshtein};
use crate::file_summary::file_summary;
use crate::snippet::{estimate_tokens, read_snippet, SnippetMode, SnippetRequest};
use crate::{callees, callers};
use ga_core::{Error, Result};
use ga_index::Store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// PR13 / S-003 AS-009 — compose a signature line from DB-driven
/// `(modifiers, name, params, return_type)` columns. Lang-agnostic
/// "approx signature for context window" form — matches what
/// `read_snippet(SnippetMode::Signature)` would extract from source,
/// minus body / docstring noise. Tools-C2 sentinel handling:
/// - empty `type_` (Tools-C2 unknown sentinel) → omit the `: type` part
/// - empty `default_value` → omit the `=value` part
/// - empty `return_type` → omit `-> return_type`
/// - empty modifiers → no leading prefix
///
/// UC consumers (minimal_context envelope, file_summary listings) use
/// this when params are populated; fall back to source-text read_snippet
/// when params == [] (Tools-C2 "no signature data" sentinel).
pub fn compose_signature_from_db(
    modifiers: &[String],
    name: &str,
    params: &[(String, String, String)],
    return_type: &str,
) -> String {
    let mut buf = String::new();
    if !modifiers.is_empty() {
        buf.push_str(&modifiers.join(" "));
        buf.push(' ');
    }
    buf.push_str(name);
    buf.push('(');
    let parts: Vec<String> = params
        .iter()
        .map(|(pn, pt, pd)| {
            let mut p = pn.clone();
            if !pt.is_empty() {
                p.push_str(": ");
                p.push_str(pt);
            }
            if !pd.is_empty() {
                p.push_str("=");
                p.push_str(pd);
            }
            p
        })
        .collect();
    buf.push_str(&parts.join(", "));
    buf.push(')');
    if !return_type.is_empty() {
        buf.push_str(" -> ");
        buf.push_str(return_type);
    }
    buf
}

/// PR13 — fetch DB-driven signature for `(name, file)` and compose via
/// `compose_signature_from_db`. Returns None if (a) Cypher fails (cache
/// missing), (b) symbol not found, (c) Tools-C2 sentinels make composition
/// no better than source: zero params AND empty return_type AND empty
/// modifiers — fall back to read_snippet so the caller-line text stays
/// useful (especially for non-function symbols where DB has nothing to
/// compose).
fn fetch_db_signature(store: &Store, name: &str, file: &str) -> Option<String> {
    let conn = store.connection().ok()?;
    let q = format!(
        "MATCH (s:Symbol {{name: '{}', file: '{}'}}) \
         RETURN s.modifiers, s.params, s.return_type LIMIT 1",
        name.replace('\'', ""),
        file.replace('\'', "")
    );
    let rs = conn.query(&q).ok()?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        let modifiers = match cols.first() {
            Some(lbug::Value::List(_, items)) => items
                .iter()
                .filter_map(|v| {
                    if let lbug::Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<String>>(),
            _ => Vec::new(),
        };
        let params: Vec<(String, String, String)> = match cols.get(1) {
            Some(lbug::Value::List(_, items)) => items
                .iter()
                .filter_map(|v| {
                    if let lbug::Value::Struct(fields) = v {
                        let mut name = String::new();
                        let mut typ = String::new();
                        let mut def = String::new();
                        for (fname, fval) in fields {
                            if let lbug::Value::String(s) = fval {
                                match fname.as_str() {
                                    "name" => name = s.clone(),
                                    "type" => typ = s.clone(),
                                    "default_value" => def = s.clone(),
                                    _ => {}
                                }
                            }
                        }
                        Some((name, typ, def))
                    } else {
                        None
                    }
                })
                .collect(),
            _ => Vec::new(),
        };
        let return_type = match cols.get(2) {
            Some(lbug::Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        // Sentinel: nothing to compose → return None to fall through.
        if params.is_empty() && return_type.is_empty() && modifiers.is_empty() {
            return None;
        }
        return Some(compose_signature_from_db(
            &modifiers,
            name,
            &params,
            &return_type,
        ));
    }
    None
}

const MAX_CALLERS: usize = 5;
const MAX_CALLEES: usize = 3;
/// Read up to N source lines per snippet (signature mode collapses by
/// language-specific heuristic; body mode caps here).
const SNIPPET_MAX_LINES: u32 = 8;
/// Levenshtein cap for AS-016 suggestions.
const SUGGESTION_LIMIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolContextReason {
    Seed,
    Caller,
    Callee,
    TypeDep,
    TestExample,
    /// Sibling module in the same directory that references the seed
    /// symbol but has no explicit Caller/Callee edge — e.g. type-position
    /// use, comment, impl bound, or a reference the indexer doesn't
    /// fully model. Added 2026-04-28 (Bug 4) for axum cross-module
    /// sibling cases (Router → method_routing.rs / path_router.rs).
    SiblingModule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolContext {
    pub symbol: String,
    pub file: String,
    pub snippet: String,
    pub reason: SymbolContextReason,
    pub tokens: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MinimalContextMeta {
    pub truncated: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MinimalContextResponse {
    pub symbols: Vec<SymbolContext>,
    pub token_estimate: u32,
    pub budget_used: f32,
    pub meta: MinimalContextMeta,
}

#[derive(Debug, Clone, Default)]
pub struct MinimalContextRequest {
    pub symbol: Option<String>,
    pub file: Option<String>,
    pub budget: u32,
    /// Optional seed-file hint for symbol mode — disambiguates generic
    /// names like `fmt`, `body`, `new` that appear in many files. When
    /// set AND a Symbol with `name == symbol AND file == seed_file_hint`
    /// exists, that match wins. Otherwise falls back to the first
    /// matching name (existing behaviour). Added 2026-04-28 for axum
    /// minimal_context bench (M3 minimal_context Hmc-gitmine).
    pub seed_file_hint: Option<String>,
}

impl MinimalContextRequest {
    pub fn for_symbol(symbol: impl Into<String>, budget: u32) -> Self {
        Self {
            symbol: Some(symbol.into()),
            file: None,
            budget,
            seed_file_hint: None,
        }
    }
    pub fn for_file(file: impl Into<String>, budget: u32) -> Self {
        Self {
            symbol: None,
            file: Some(file.into()),
            budget,
            seed_file_hint: None,
        }
    }
    /// Symbol mode + seed-file hint. Use when the symbol name is generic
    /// (`fmt` / `body` / `new`) and you know the file the caller meant.
    pub fn for_symbol_in_file(
        symbol: impl Into<String>,
        seed_file: impl Into<String>,
        budget: u32,
    ) -> Self {
        Self {
            symbol: Some(symbol.into()),
            file: None,
            budget,
            seed_file_hint: Some(seed_file.into()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────

pub fn minimal_context(
    store: &Store,
    req: &MinimalContextRequest,
) -> Result<MinimalContextResponse> {
    let symbol = req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let file = req.file.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let budget = req.budget;

    let seed_file_hint = req
        .seed_file_hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match (symbol, file) {
        (Some(sym), _) => minimal_context_for_symbol(store, sym, budget, seed_file_hint),
        (None, Some(f)) => minimal_context_for_file(store, f, budget),
        (None, None) => Err(Error::InvalidParams(
            "ga_minimal_context: at least one of `symbol` or `file` is required".to_string(),
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Symbol mode (AS-005, AS-007, AS-016)
// ─────────────────────────────────────────────────────────────────────────

fn minimal_context_for_symbol(
    store: &Store,
    symbol: &str,
    budget: u32,
    seed_file_hint: Option<&str>,
) -> Result<MinimalContextResponse> {
    if !is_safe_ident(symbol) {
        return Err(Error::InvalidParams(format!(
            "ga_minimal_context: `symbol` must be a valid identifier, got `{symbol}`"
        )));
    }

    let repo_root: PathBuf = store.metadata().repo_root.clone().into();
    let seed_def = find_symbol_def_with_hint(store, symbol, seed_file_hint)?;
    let Some(def) = seed_def else {
        let suggestions = nearest_symbol_names(store, symbol)?;
        return Err(Error::SymbolNotFound { suggestions });
    };

    let mut contexts: Vec<SymbolContext> = Vec::new();
    let mut tokens_used: u32 = 0;
    let mut truncated = false;

    // (1) Seed — full body (clamped to SNIPPET_MAX_LINES).
    let seed_snip = read_snippet(
        &repo_root,
        &SnippetRequest {
            file: def.file.clone(),
            line: def.line,
            max_lines: SNIPPET_MAX_LINES,
            mode: SnippetMode::Body,
        },
    )?;
    if !seed_snip.text.is_empty() {
        let t = estimate_tokens(&seed_snip.text);
        if tokens_used + t <= budget {
            contexts.push(SymbolContext {
                symbol: symbol.to_string(),
                file: def.file.clone(),
                snippet: seed_snip.text,
                reason: SymbolContextReason::Seed,
                tokens: t,
            });
            tokens_used += t;
        } else {
            // Try signature-only fallback.
            let sig_snip = read_snippet(
                &repo_root,
                &SnippetRequest {
                    file: def.file.clone(),
                    line: def.line,
                    max_lines: SNIPPET_MAX_LINES,
                    mode: SnippetMode::Signature,
                },
            )?;
            let truncated_text =
                truncate_to_budget(&sig_snip.text, budget.saturating_sub(tokens_used));
            let t = estimate_tokens(&truncated_text);
            if !truncated_text.is_empty() {
                contexts.push(SymbolContext {
                    symbol: symbol.to_string(),
                    file: def.file.clone(),
                    snippet: truncated_text,
                    reason: SymbolContextReason::Seed,
                    tokens: t,
                });
                tokens_used += t;
            }
            truncated = true;
        }
    }

    // (2) Callers — signature only, top MAX_CALLERS.
    let callers_resp = callers(store, symbol, None)?;
    for caller in callers_resp.callers.into_iter().take(MAX_CALLERS) {
        // PR13 / AS-009 — prefer DB-driven signature when populated.
        // Tools-C2 fallback: empty params + empty return_type → use source.
        let db_snip = fetch_db_signature(store, &caller.symbol, &caller.file);
        let text = match db_snip {
            Some(s) if !s.is_empty() => s,
            _ => {
                let snip = read_snippet(
                    &repo_root,
                    &SnippetRequest {
                        file: caller.file.clone(),
                        line: caller.line,
                        max_lines: SNIPPET_MAX_LINES,
                        mode: SnippetMode::Signature,
                    },
                )?;
                snip.text
            }
        };
        if text.is_empty() {
            continue;
        }
        let t = estimate_tokens(&text);
        if tokens_used + t > budget {
            truncated = true;
            break;
        }
        contexts.push(SymbolContext {
            symbol: caller.symbol,
            file: caller.file,
            snippet: text,
            reason: SymbolContextReason::Caller,
            tokens: t,
        });
        tokens_used += t;
    }

    // (3) Callees — signature only, top MAX_CALLEES.
    let callees_resp = callees(store, symbol, None)?;
    for callee in callees_resp.callees.into_iter().take(MAX_CALLEES) {
        if callee.external {
            continue;
        }
        // PR13 / AS-009 — same DB-first / source-fallback pattern as callers.
        let db_snip = fetch_db_signature(store, &callee.symbol, &callee.file);
        let text = match db_snip {
            Some(s) if !s.is_empty() => s,
            _ => {
                let snip = read_snippet(
                    &repo_root,
                    &SnippetRequest {
                        file: callee.file.clone(),
                        line: callee.line,
                        max_lines: SNIPPET_MAX_LINES,
                        mode: SnippetMode::Signature,
                    },
                )?;
                snip.text
            }
        };
        if text.is_empty() {
            continue;
        }
        let t = estimate_tokens(&text);
        if tokens_used + t > budget {
            truncated = true;
            break;
        }
        contexts.push(SymbolContext {
            symbol: callee.symbol,
            file: callee.file,
            snippet: text,
            reason: SymbolContextReason::Callee,
            tokens: t,
        });
        tokens_used += t;
    }

    // 2026-05-02 audit removed `surface_reexport_sites` (parent-dir scan +
    // regex match for barrel re-exports). A/B test on all 6 M3 fixtures
    // showed 0 contribution on baseline (re-export off, sibling on = same
    // score) and slight negative on gin (-0.008). Bench-tuned to axum;
    // current indexer doesn't model REEXPORTS edges so the workaround
    // walked the file tree at query time. Universal fix is REEXPORTS edge
    // type in indexer schema — backlog (DEADCODE-2 / ARCH-1 share scope).

    // (3) Sibling-module discovery — INTERIM workaround for Rust mod-tree
    // cross-file references. When seed file is `<dir>/<seed>.rs`, scan
    // sibling `*.rs` files for word-boundary occurrences and surface as
    // SiblingModule reason. Rust-only: other languages route via re-export
    // / importer / test-example buckets. A/B test 2026-05-02 confirmed
    // load-bearing on axum (-0.083 if removed alone). Backlog: improve
    // ga-parser Rust extraction to capture type-position uses + impl
    // bounds + indirect `super::Foo` paths as REFERENCES edges, then this
    // step retires.
    surface_sibling_modules(
        &repo_root,
        &def.file,
        symbol,
        budget,
        &mut tokens_used,
        &mut truncated,
        &mut contexts,
    )?;

    // (4) Imported types — AS-005 §Data priority 4. Read the seed file's
    // imports and surface them as a single TypeDep context block when
    // budget allows. Heuristic: prune to imports whose name appears in
    // the seed snippet (signature contains type annotations referencing
    // them) — keeps recall focused, avoids dumping unrelated imports.
    if let Ok(fs) = file_summary(store, &def.file) {
        if !fs.imports.is_empty() {
            let seed_text = contexts
                .iter()
                .find(|c| c.reason == SymbolContextReason::Seed)
                .map(|c| c.snippet.clone())
                .unwrap_or_default();
            let relevant: Vec<String> = fs
                .imports
                .iter()
                .filter(|imp| {
                    imp.split(&['.', '/', ':'][..])
                        .any(|seg| !seg.is_empty() && seed_text.contains(seg))
                })
                .cloned()
                .collect();
            let imports_to_show: Vec<String> = if relevant.is_empty() {
                fs.imports.iter().take(5).cloned().collect()
            } else {
                relevant.into_iter().take(8).collect()
            };
            if !imports_to_show.is_empty() {
                let snippet_text = imports_to_show.join("\n") + "\n";
                let t = estimate_tokens(&snippet_text);
                if tokens_used + t <= budget {
                    contexts.push(SymbolContext {
                        symbol: "<imports>".to_string(),
                        file: def.file.clone(),
                        snippet: snippet_text,
                        reason: SymbolContextReason::TypeDep,
                        tokens: t,
                    });
                    tokens_used += t;
                } else {
                    truncated = true;
                }
            }
        }
    }

    // (5) Test examples — find canonical test companion(s) for the
    // seed symbol. Strategy depends on whether seed itself is a test:
    //
    //   Production seed (e.g. `user_perm_str`) → look for one of:
    //     - `test_<seed>` (Python: pytest, unittest)
    //     - `Test<PascalCase(seed)>` (Go: testing pkg convention)
    //     - `Test<seed>` literal (TS/JS Jest sometimes)
    //     Filter to is_test_path. Stop at first hit.
    //
    //   Test seed (already starts with `test_` or `Test`) → look for
    //     other tests in the SAME file (sibling tests typically share
    //     setup and exercise related behaviour, which matches tasks-v6
    //     must_touch_symbols pattern observed on gin).
    //
    // Discovery: M3 audit on django/gin, 2026-04-28 — production seed
    // tasks missed test companions ~50% of the time before this step;
    // gin test-seed tasks expected related siblings as must_touch.
    {
        let conn = store
            .connection()
            .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
        let seed_is_test = symbol.starts_with("test_")
            || (symbol.starts_with("Test")
                && symbol
                    .chars()
                    .nth(4)
                    .map(|c| c.is_ascii_uppercase() || c == '_')
                    .unwrap_or(false));
        let candidate_names: Vec<String> = if seed_is_test {
            // Sibling test discovery — limit set, computed below from file.
            Vec::new()
        } else {
            // Convention candidates for production seed.
            let mut v = vec![format!("test_{symbol}")];
            // PascalCase for Go: trySetUsingParser → TrySetUsingParser
            // → Test prefix → TestTrySetUsingParser
            let mut cap = symbol.chars();
            if let Some(first) = cap.next() {
                let pascal: String = first.to_ascii_uppercase().to_string() + cap.as_str();
                v.push(format!("Test{pascal}"));
            }
            v
        };

        let test_candidates: Vec<(String, String, u32)> = if seed_is_test {
            // Sibling-test discovery: same DIR as seed, not just same
            // file. In Go (gin/etc.) tests for one package live across
            // `<x>_test.go`, `<y>_test.go` in the same dir; tasks-v6
            // GT often expects siblings across these files. Same dir
            // = same package = related tests.
            let dir_prefix = std::path::Path::new(&def.file)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Match files whose path starts with the seed's parent dir
            // followed by `/` (sibling files only, not subdirs deeper
            // than 0). For root-level files (dir_prefix=""), match
            // files without `/`. LIMIT 1000 — gin's context_test.go
            // alone has 200+ test functions; previous LIMIT 200 cut off
            // the canonical PDF/JSON sibling tests before sorting could
            // pick them up.
            let cypher = if dir_prefix.is_empty() {
                // Root-level seed file: match files where s.file has no
                // `/` (single-segment path).
                "MATCH (s:Symbol) WHERE s.kind <> 'external' \
                 AND NOT s.file CONTAINS '/' \
                 RETURN s.name, s.file, s.line LIMIT 1000"
                    .to_string()
            } else {
                format!(
                    "MATCH (s:Symbol) WHERE s.kind <> 'external' \
                     AND s.file STARTS WITH '{}/' \
                     RETURN s.name, s.file, s.line LIMIT 1000",
                    dir_prefix.replace('\'', "''")
                )
            };
            conn.query(&cypher)
                .map(|rs| {
                    rs.into_iter()
                        .filter_map(|row| {
                            let cols: Vec<lbug::Value> = row.into_iter().collect();
                            if cols.len() < 3 {
                                return None;
                            }
                            let name = match &cols[0] {
                                lbug::Value::String(s) => s.clone(),
                                _ => return None,
                            };
                            if name == symbol {
                                return None; // skip seed itself
                            }
                            // Only accept test-named symbols.
                            let is_test = name.starts_with("test_")
                                || (name.starts_with("Test")
                                    && name
                                        .chars()
                                        .nth(4)
                                        .map(|c| c.is_ascii_uppercase() || c == '_')
                                        .unwrap_or(false));
                            if !is_test {
                                return None;
                            }
                            let file = match &cols[1] {
                                lbug::Value::String(s) => s.clone(),
                                _ => return None,
                            };
                            // Same-dir sibling guard: drop if file is
                            // in a deeper sub-directory than seed
                            // (we want siblings, not descendants).
                            let file_parent = std::path::Path::new(&file)
                                .parent()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if file_parent != dir_prefix {
                                return None;
                            }
                            let line = match &cols[2] {
                                lbug::Value::Int64(n) => *n as u32,
                                _ => return None,
                            };
                            Some((name, file, line))
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            // Convention-name lookup for production seeds.
            let mut acc: Vec<(String, String, u32)> = Vec::new();
            for cand in &candidate_names {
                let cypher = format!(
                    "MATCH (s:Symbol) WHERE s.name = '{}' AND s.kind <> 'external' \
                     RETURN s.name, s.file, s.line LIMIT 5",
                    cand.replace('\'', "''")
                );
                if let Ok(rs) = conn.query(&cypher) {
                    for row in rs {
                        let cols: Vec<lbug::Value> = row.into_iter().collect();
                        if cols.len() < 3 {
                            continue;
                        }
                        let name = match &cols[0] {
                            lbug::Value::String(s) => s.clone(),
                            _ => continue,
                        };
                        let file = match &cols[1] {
                            lbug::Value::String(s) => s.clone(),
                            _ => continue,
                        };
                        let line = match &cols[2] {
                            lbug::Value::Int64(n) => *n as u32,
                            _ => continue,
                        };
                        if !crate::common::is_test_path(&file) {
                            continue;
                        }
                        acc.push((name, file, line));
                    }
                }
                if !acc.is_empty() {
                    break; // first matching convention wins
                }
            }
            acc
        };

        // Sort sibling candidates by relatedness to seed:
        //   1. Longest-common-prefix (more shared prefix = more related)
        //   2. Trailing-chunk match (e.g. seed ends in "PDF" → siblings
        //      also containing "PDF" rank higher).
        // Composite key keeps the close-matches at the front of the
        // take(6) cap so e.g. TestContextRenderPDF picks
        // TestContextRenderNoContentPDF (shares "PDF" suffix) over
        // TestContextRenderPureJSON (shares longer prefix but
        // different feature).
        let mut sorted_candidates = test_candidates;
        if seed_is_test {
            let seed_tail = trailing_camel_chunk(symbol);
            sorted_candidates.sort_by_key(|(name, _, _)| {
                let lcp = name
                    .chars()
                    .zip(symbol.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                let tail_match = if !seed_tail.is_empty() && name.contains(&seed_tail) {
                    1
                } else {
                    0
                };
                // Descending — trailing match dominates lcp.
                std::cmp::Reverse((tail_match, lcp))
            });
        }

        // Emit up to 6 test contexts. Production-symbol seeds typically
        // have 1 canonical companion (`test_<name>`); test-symbol seeds
        // (gin / Go style) often have 5-8 sibling tests in the same
        // file (TestContextRender*, TestContextGet*, etc.). Cap at 6
        // to fit budget on large signature snippets.
        for (name, file, line) in sorted_candidates.into_iter().take(6) {
            let snip = read_snippet(
                &repo_root,
                &SnippetRequest {
                    file: file.clone(),
                    line,
                    max_lines: SNIPPET_MAX_LINES,
                    mode: SnippetMode::Signature,
                },
            )?;
            if snip.text.is_empty() {
                continue;
            }
            let t = estimate_tokens(&snip.text);
            if tokens_used + t > budget {
                truncated = true;
                break;
            }
            contexts.push(SymbolContext {
                symbol: name,
                file,
                snippet: snip.text,
                reason: SymbolContextReason::TestExample,
                tokens: t,
            });
            tokens_used += t;
        }
    }

    let warning = if budget == 0 {
        Some("budget too small for full signature".to_string())
    } else if truncated {
        // AS-007 §Then: exact warning string `"budget too small for full signature"`.
        Some("budget too small for full signature".to_string())
    } else {
        None
    };

    let budget_used = if budget == 0 {
        0.0
    } else {
        tokens_used as f32 / budget as f32
    };

    Ok(MinimalContextResponse {
        symbols: contexts,
        token_estimate: tokens_used,
        budget_used,
        meta: MinimalContextMeta {
            truncated: truncated || budget == 0,
            warning,
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────
// File mode (AS-006)
// ─────────────────────────────────────────────────────────────────────────

fn minimal_context_for_file(
    store: &Store,
    file: &str,
    budget: u32,
) -> Result<MinimalContextResponse> {
    let repo_root: PathBuf = store.metadata().repo_root.clone().into();
    let symbols = list_file_symbols(store, file)?;
    if symbols.is_empty() {
        return Err(Error::InvalidParams(format!(
            "file `{file}` not found in index or contains no symbols"
        )));
    }

    let mut contexts: Vec<SymbolContext> = Vec::new();
    let mut tokens_used: u32 = 0;
    let mut truncated = false;

    // AS-006 §Then: include top-level imports of the file at the top of
    // the returned context. file_summary.imports only tracks repo-local
    // IMPORTS edges (stdlib excluded by graph design), so for display we
    // read the raw source and grep top-level `import`/`use`/`require`
    // lines — covers stdlib (`import hashlib`) plus repo-local imports
    // uniformly. Emit as a single TypeDep block.
    let raw_imports = read_top_level_imports(&repo_root, file);
    if !raw_imports.is_empty() {
        let snippet_text = raw_imports
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let t = estimate_tokens(&snippet_text);
        if tokens_used + t <= budget {
            contexts.push(SymbolContext {
                symbol: "<imports>".to_string(),
                file: file.to_string(),
                snippet: snippet_text,
                reason: SymbolContextReason::TypeDep,
                tokens: t,
            });
            tokens_used += t;
        } else {
            truncated = true;
        }
    }

    for sym in symbols {
        // Skip private helpers when budget binds: heuristic = leading
        // underscore (Python/JS convention) or `private`/`internal`
        // qualifier in declared kind. AS-006 §Then "drops private helpers
        // + method bodies when budget binds".
        //
        // AS-006 §Then "class docstrings" — for Class-kind symbols, read a
        // wider window in Body mode and append the docstring (if present)
        // to the signature. For Function/Method, signature-only suffices.
        let is_class = matches!(
            sym.kind.as_str(),
            "class" | "interface" | "trait" | "struct" | "enum"
        );
        let mode = if is_class {
            SnippetMode::Body
        } else {
            SnippetMode::Signature
        };
        let max_lines = if is_class { 6 } else { SNIPPET_MAX_LINES };
        let snip = read_snippet(
            &repo_root,
            &SnippetRequest {
                file: file.to_string(),
                line: sym.line,
                max_lines,
                mode,
            },
        )?;
        if snip.text.is_empty() {
            continue;
        }
        // For classes, trim body to keep declaration line + docstring (if any).
        let final_text = if is_class {
            extract_class_signature_with_docstring(&snip.text)
        } else {
            snip.text
        };
        let t = estimate_tokens(&final_text);
        if tokens_used + t > budget {
            truncated = true;
            // Don't break — give the next-priority symbol a chance.
            // Private helpers were already implicitly de-prioritized by
            // ordering (we put them last via name sort).
            continue;
        }
        contexts.push(SymbolContext {
            symbol: sym.name,
            file: file.to_string(),
            snippet: final_text,
            reason: SymbolContextReason::Seed,
            tokens: t,
        });
        tokens_used += t;
    }

    let warning = if budget == 0 {
        Some("budget is 0 — no content fits".to_string())
    } else if truncated {
        Some("budget too small to include all file symbols; partial returned".to_string())
    } else {
        None
    };
    let budget_used = if budget == 0 {
        0.0
    } else {
        tokens_used as f32 / budget as f32
    };

    Ok(MinimalContextResponse {
        symbols: contexts,
        token_estimate: tokens_used,
        budget_used,
        meta: MinimalContextMeta {
            truncated: truncated || budget == 0,
            warning,
        },
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers — graph queries
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SymbolDef {
    name: String,
    file: String,
    line: u32,
    kind: String,
}

/// Variant of `find_symbol_def` that honours a seed-file hint strictly.
/// When `seed_file_hint` is set, ONLY a `(name == symbol AND file == hint)`
/// match is accepted — a miss returns `Ok(None)` and the caller surfaces
/// SymbolNotFound. Falling back to a global lookup would hide stale GT
/// hints + indexer extraction gaps under a wrong-file seed (this was the
/// regex audit failure mode — see /mf-voices Round 4 consensus).
///
/// No-hint path delegates to existing `find_symbol_def` — global lookup
/// is the documented behaviour when callers don't supply a hint.
fn find_symbol_def_with_hint(
    store: &Store,
    symbol: &str,
    seed_file_hint: Option<&str>,
) -> Result<Option<SymbolDef>> {
    let Some(hint) = seed_file_hint else {
        return find_symbol_def(store, symbol);
    };
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{}' AND s.file = '{}' \
         AND s.kind <> 'external' \
         RETURN s.name, s.file, s.line, s.kind LIMIT 1",
        symbol.replace('\'', "''"),
        hint.replace('\'', "''")
    );
    let Ok(rs) = conn.query(&cypher) else {
        // Cypher error (not "0 rows") is opaque — surface as None too.
        return Ok(None);
    };
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let file = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let kind = match &cols[3] {
            lbug::Value::String(s) => s.clone(),
            _ => String::new(),
        };
        return Ok(Some(SymbolDef {
            name,
            file,
            line,
            kind,
        }));
    }
    Ok(None)
}

fn find_symbol_def(store: &Store, symbol: &str) -> Result<Option<SymbolDef>> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.name = '{symbol}' AND s.kind <> 'external' \
         RETURN s.name, s.file, s.line, s.kind LIMIT 1"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("find_symbol_def: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let file = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let kind = match &cols[3] {
            lbug::Value::String(s) => s.clone(),
            _ => String::new(),
        };
        return Ok(Some(SymbolDef {
            name,
            file,
            line,
            kind,
        }));
    }
    Ok(None)
}

fn list_file_symbols(store: &Store, file: &str) -> Result<Vec<SymbolDef>> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let cypher = format!(
        "MATCH (s:Symbol) WHERE s.file = '{file}' AND s.kind <> 'external' \
         RETURN s.name, s.file, s.line, s.kind"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("list_file_symbols: {e}")))?;
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 4 {
            continue;
        }
        let name = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let file = match &cols[1] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let line = match &cols[2] {
            lbug::Value::Int64(n) => *n as u32,
            _ => 0,
        };
        let kind = match &cols[3] {
            lbug::Value::String(s) => s.clone(),
            _ => String::new(),
        };
        out.push(SymbolDef {
            name,
            file,
            line,
            kind,
        });
    }
    // Sort: public-prefixed names first (no leading underscore), then by
    // name lex. Honors AS-006 "drops private helpers when budget binds".
    out.sort_by(|a, b| {
        let a_priv = a.name.starts_with('_');
        let b_priv = b.name.starts_with('_');
        a_priv.cmp(&b_priv).then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn nearest_symbol_names(store: &Store, target: &str) -> Result<Vec<String>> {
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN DISTINCT s.name")
        .map_err(|e| Error::Other(anyhow::anyhow!("nearest_symbol_names: {e}")))?;
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

/// AS-006 — Extract class declaration line + docstring (if present).
/// For Python `class Foo:\n    """docs"""\n    ...`: keep the `class` line
/// + the immediately-following triple-quoted docstring block.
/// For brace-langs (Rust/TS/Java/Kotlin/C#): keep declaration line(s) up
/// to and including the opening `{`; docstrings are typically separate
/// `///` / `/**` comments above the class — those would already be on
/// preceding lines and require a different read window. v1 ships
/// Python-style docstring extraction; brace-lang doc-comment extraction
/// deferred to future iteration.
fn extract_class_signature_with_docstring(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let first = lines[0];
    let mut out = String::new();
    out.push_str(first);
    out.push('\n');

    // Brace-lang: signature ends at `{`; if declaration spans multiple
    // lines, capture them all up to and including that brace.
    if first.contains('{') || (!first.trim_end().ends_with(':') && !first.contains("class")) {
        // brace-lang or non-Python — first line is the declaration.
        for line in lines.iter().skip(1) {
            out.push_str(line);
            out.push('\n');
            if line.contains('{') {
                break;
            }
        }
        return out;
    }

    // Python-style: look for triple-quoted docstring immediately after.
    let mut in_docstring = false;
    let mut quote: Option<&str> = None;
    for line in lines.iter().skip(1) {
        let trimmed = line.trim_start();
        if !in_docstring {
            // Either a docstring open or the first body line — stop if
            // it's not a docstring.
            if trimmed.starts_with("\"\"\"") {
                quote = Some("\"\"\"");
            } else if trimmed.starts_with("'''") {
                quote = Some("'''");
            } else {
                // No docstring — return just the class declaration line.
                return out;
            }
            out.push_str(line);
            out.push('\n');
            in_docstring = true;
            // Single-line docstring: opens and closes on same line.
            let q = quote.unwrap();
            let after_open = trimmed.trim_start_matches(q);
            if after_open.contains(q) {
                return out;
            }
            continue;
        }
        // Inside multi-line docstring — scan for close.
        out.push_str(line);
        out.push('\n');
        if let Some(q) = quote {
            if trimmed.contains(q) {
                return out;
            }
        }
    }
    out
}

/// AS-006 — Read top-level import lines from a file's raw source.
/// Returns the literal lines (preserving order). Stops at the first non-
/// import / non-blank / non-comment line — top-level imports are
/// conventionally clustered at the top of a file across all v1.1 langs.
///
/// Per-lang prefixes recognized:
/// - Python: `import ` / `from `
/// - JS/TS:  `import ` / `export ... from`  (ES module — re-exports count)
/// - Rust:   `use `
/// - Go:     `import ` (single-line and `import (...)` group block both)
/// - Java/Kotlin: `import `
/// - C#:     `using `
/// - Ruby:   `require ` / `require_relative `
/// Unknown ext → conservative: any line containing `import` token.
fn read_top_level_imports(repo_root: &std::path::Path, file: &str) -> Vec<String> {
    let path = if std::path::Path::new(file).is_absolute() {
        std::path::PathBuf::from(file)
    } else {
        repo_root.join(file)
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut out = Vec::new();
    let mut in_go_import_block = false;
    let mut seen_non_import = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        // Skip blanks + comments at the top of the file.
        if trimmed.is_empty() {
            continue;
        }
        let is_comment =
            trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*");
        if is_comment && !seen_non_import {
            continue;
        }

        // Go group-import: `import (\n  "a"\n  "b"\n)` — capture the lines
        // inside the parentheses verbatim.
        if ext == "go" {
            if trimmed.starts_with("import (") {
                in_go_import_block = true;
                out.push(line.to_string());
                continue;
            }
            if in_go_import_block {
                out.push(line.to_string());
                if trimmed.starts_with(')') {
                    in_go_import_block = false;
                }
                continue;
            }
        }

        let is_import = match ext {
            "py" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
                trimmed.starts_with("import ") || trimmed.starts_with("export ")
            }
            "rs" => trimmed.starts_with("use ") || trimmed.starts_with("pub use "),
            "go" => trimmed.starts_with("import \""),
            "java" | "kt" | "kts" => trimmed.starts_with("import "),
            "cs" => trimmed.starts_with("using ") && !trimmed.starts_with("using ("),
            "rb" => trimmed.starts_with("require ") || trimmed.starts_with("require_relative "),
            _ => trimmed.contains("import"),
        };
        if is_import {
            out.push(line.to_string());
        } else {
            // Stop at first non-import — only top-level cluster matters.
            seen_non_import = true;
            // For Python, allow `__all__` / `__future__` lines after imports.
            // Otherwise break.
            if ext == "py" && (trimmed.starts_with("__all__") || trimmed.starts_with("__version__"))
            {
                continue;
            }
            break;
        }
    }
    out
}

/// Trim text to fit token budget while keeping the head (declaration line).
/// Extract the trailing CamelCase chunk from a symbol name. For
/// `TestContextRenderPDF` returns `"PDF"`; for `getUserName` returns
/// `"Name"`; for `test_user_perm_str` returns `"str"`. Used to rank
/// sibling-test candidates by suffix similarity.
///
/// Algorithm: find the last position `i` where a word boundary
/// happens — either `lowercase → uppercase` (camelCase boundary) or
/// `_` (snake_case boundary). Trailing chunk = `name[boundary..]`.
fn trailing_camel_chunk(name: &str) -> String {
    let bytes = name.as_bytes();
    if bytes.len() < 2 {
        return name.to_string();
    }
    let mut last_boundary = 0usize;
    for i in 1..bytes.len() {
        let prev = bytes[i - 1];
        let cur = bytes[i];
        if prev == b'_' {
            last_boundary = i;
        } else if prev.is_ascii_lowercase() && cur.is_ascii_uppercase() {
            last_boundary = i;
        }
    }
    name[last_boundary..].trim_start_matches('_').to_string()
}

fn truncate_to_budget(text: &str, budget: u32) -> String {
    if budget == 0 {
        return String::new();
    }
    let est = estimate_tokens(text);
    if est <= budget {
        return text.to_string();
    }
    // Char-budget reverse-derive: if 4 chars ≈ 1 token, keep budget*4 chars
    // (with some headroom). This satisfies Tools-C3 ±10% slack.
    let max_chars = (budget as usize).saturating_mul(4);
    let truncated: String = text.chars().take(max_chars).collect();
    truncated
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ─────────────────────────────────────────────────────────────────────────
// Sibling-module discovery (Step 3b — Bug 4)
// ─────────────────────────────────────────────────────────────────────────

const SIBLING_MAX_HITS: usize = 3;
const SIBLING_FILE_MAX_BYTES: usize = 64 * 1024;

fn surface_sibling_modules(
    repo_root: &std::path::Path,
    seed_file: &str,
    symbol: &str,
    budget: u32,
    tokens_used: &mut u32,
    truncated: &mut bool,
    contexts: &mut Vec<SymbolContext>,
) -> Result<()> {
    // Scope: same-dir Rust siblings. JS/TS/Py have their own discovery
    // patterns (re-export, importer, test-example) that already cover
    // this. Limiting to .rs avoids false hits in language-mixed dirs.
    let seed_path = std::path::Path::new(seed_file);
    let is_rust = seed_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "rs")
        .unwrap_or(false);
    if !is_rust {
        return Ok(());
    }
    let Some(dir) = seed_path.parent() else {
        return Ok(());
    };
    let abs_dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        repo_root.join(dir)
    };
    let Ok(entries) = std::fs::read_dir(&abs_dir) else {
        return Ok(());
    };

    // (file_rel, occurrence_count) for every .rs sibling that mentions
    // `symbol`. Skip the seed file itself + any file already in contexts.
    let already: std::collections::BTreeSet<String> =
        contexts.iter().map(|c| c.file.clone()).collect();
    let mut hits: Vec<(std::path::PathBuf, u32)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = match path.strip_prefix(repo_root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => path.clone(),
        };
        let rel_str = rel.to_string_lossy().into_owned();
        if rel_str == seed_file {
            continue;
        }
        if already.contains(&rel_str) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() == 0 || meta.len() as usize > SIBLING_FILE_MAX_BYTES {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let count = count_word_boundary_occurrences(&body, symbol);
        if count > 0 {
            hits.push((rel, count));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    // Rank by occurrence count (desc), then by path lex (stable).
    hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hits.truncate(SIBLING_MAX_HITS);

    for (rel, _count) in hits {
        let abs = repo_root.join(&rel);
        let Ok(body) = std::fs::read_to_string(&abs) else {
            continue;
        };
        // Snippet — first ~30 lines of the sibling file. Same shape as
        // ReExport contexts; user gets enough surrounding API to see how
        // the sibling uses the seed.
        let snippet: String = body.lines().take(30).collect::<Vec<_>>().join("\n");
        let snippet = if snippet.len() > 1500 {
            truncate_to_budget(&snippet, 800 / 4)
        } else {
            snippet
        };
        let t = estimate_tokens(&snippet);
        if *tokens_used + t > budget {
            *truncated = true;
            return Ok(());
        }
        contexts.push(SymbolContext {
            symbol: symbol.to_string(),
            file: rel.to_string_lossy().into_owned(),
            snippet,
            reason: SymbolContextReason::SiblingModule,
            tokens: t,
        });
        *tokens_used += t;
    }
    Ok(())
}

fn count_word_boundary_occurrences(body: &str, needle: &str) -> u32 {
    if needle.is_empty() {
        return 0;
    }
    let bytes = body.as_bytes();
    let n = needle.as_bytes();
    let mut count = 0u32;
    let mut i = 0;
    while i + n.len() <= bytes.len() {
        if &bytes[i..i + n.len()] == n {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_ok = i + n.len() == bytes.len() || !is_ident_byte(bytes[i + n.len()]);
            if before_ok && after_ok {
                count += 1;
            }
        }
        i += 1;
    }
    count
}
