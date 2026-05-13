//! Indexer pipeline — composes walker + parser + bulk graph writer.
//!
//! Called by Tools S-001+ to populate a `Store` with File + Symbol nodes +
//! DEFINES edges before queries can answer. Strategy mirrors the AS-018 spike
//! driver: generate CSV files in a tempdir, then `COPY ... FROM '<csv>'` into
//! lbug for bulk load (~100× faster than per-row MERGE).

use crate::common;
use crate::import_resolve::{resolve_pending_imports, ImportRow, PendingImport};
use anyhow::{anyhow, Context, Result as AResult};
use ga_index::Store;
use ga_parser::calls::extract_calls_from_tree;
use ga_parser::extends::extract_extends_from_tree;
use ga_parser::imports::extract_imports_from_tree;
use ga_parser::limits::{parse_file_tree, LimitConfig, ParseTreeOutcome};
use ga_parser::references::extract_references_from_tree;
use ga_parser::walk::walk_repo;
use ga_parser::ParserPool;
use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub files: usize,
    pub symbols: usize,
    pub defines_edges: usize,
    pub calls_edges: usize,
    pub imports_edges: usize,
    pub extends_edges: usize,
    pub references_edges: usize,
    pub module_typed_edges: usize,
    /// Tools-C4 — count of `qualified_name` collisions resolved via
    /// `#dup<N>` suffix. AS-006 / Tools-C5.
    pub qualified_name_collision_count: usize,
    /// Tools-C4 — count of `import { name }` entries that resolved to a
    /// target file but the named symbol wasn't found in that file
    /// (PR7 IMPORTS_NAMED resolution drop). Distinct from external
    /// imports (those don't resolve a target file at all).
    pub unresolved_imports_count: usize,
    /// Tools-C4 — count of decorator/annotation names that didn't
    /// resolve to a Symbol (PR8 DECORATES drop, e.g. stdlib
    /// `@functools.lru_cache`). AS-015.
    pub unresolved_decorators_count: usize,
    /// v1.4 (S-001a / Tools-C4) — count of `is_override=true` symbols
    /// whose parent method couldn't be resolved in-repo (vendored /
    /// external base classes). Surface for the writer-without-reader
    /// audit gate (AS-012 mockito rate ≥5%).
    pub unresolved_overrides_count: usize,
    /// v1.4 (S-001a) — count of pathological self-override cases
    /// (parser/qualified-name collision producing same-id pair). AS-003
    /// guard — incrementing this means an upstream parser bug.
    pub self_override_skip_count: usize,
}

/// Build (or rebuild) the graph for `repo_root` into `store`. Idempotent —
/// safe to call multiple times on the same store.
pub fn build_index(store: &Store, repo_root: &Path) -> AResult<IndexStats> {
    // Walk the repo for source files.
    let report = walk_repo(repo_root).map_err(|e| anyhow!("walk_repo failed: {e}"))?;
    if report.entries.is_empty() {
        return Ok(IndexStats::default());
    }

    // Parse each file; collect (rel_path, lang, size, symbols, calls).
    // Parse-once optimization: one tree-sitter parse per file shared across
    // walker (symbols) + 4 extractors (calls, references, extends, imports).
    // Pre-refactor each extractor re-parsed independently → 5× cost; this
    // path drops parse-phase time ~80% on large repos. ParserPool is
    // reused across all files in this loop instead of re-allocating per
    // call (was 5 pool allocations per file pre-refactor).
    let cfg = LimitConfig::from_env();
    let pool = ParserPool::new();
    let mut file_rows: Vec<FileRow> = Vec::new();
    let mut symbol_rows: Vec<SymbolRow> = Vec::new();
    let mut defines_rows: Vec<(String, String)> = Vec::new();
    let mut symbol_ids_seen: HashSet<String> = HashSet::new();
    // Per-file: raw calls (enclosing_symbol_name → callee_name). Resolved
    // in a second pass once all symbols are known (name-based lookup).
    let mut pending_calls: Vec<PendingCall> = Vec::new();
    let mut pending_imports: Vec<PendingImport> = Vec::new();
    // (class_file, class_name, class_line, base_name) — resolved after all
    // files parse so cross-file `class B(A)` can find `A` wherever it lives.
    let mut pending_extends: Vec<PendingExtends> = Vec::new();
    // Foundation-C15 — value-reference sites collected during parse, resolved
    // to (caller_id, target_id) pairs after all symbols known (like CALLS).
    let mut pending_refs: Vec<PendingRef> = Vec::new();
    // Module-scope TypePosition refs (impl Trait, static/const type annotations)
    // have no enclosing function. Stored as (file_path, target_id) for
    // MODULE_TYPED edges so dead_code doesn't flag the target as 0-in-degree.
    let mut module_typed_rows: Vec<(String, String)> = Vec::new();
    // KG-9 — (method_id, file, enclosing_class_name) captured during symbol
    // iteration. Resolved after symbol_by_file_name is fully built so the
    // class lookup always hits — class might be parsed after its members in
    // pathological source order (rare but possible).
    let mut pending_contains: Vec<(String, String, String)> = Vec::new();
    // Per-file name → id map for within-file resolution. (rel, name) → id.
    // When a name has multiple defs in the same file (overloads rare in our
    // 5 langs), first-write wins; polymorphic-confidence handling lands in
    // cluster E.
    let mut symbol_by_file_name: HashMap<(String, String), String> = HashMap::new();
    // PR8 — pending DECORATES edges. Resolved after symbol_by_file_name +
    // symbol_by_name are fully built. Each entry: (decorated_symbol_id,
    // decorated_file, decorator_name).
    // Gap 6 — pending_decorates tuple: (decorated_id, decorated_file,
    // decorator_name, decorator_args). args is raw paren-stripped source
    // text from Python `extract_decorator_args` (empty for no-arg
    // decorators / Java/C# annotations / Ruby synthetic). Tools-C14
    // sanitizer applies at CSV emit.
    let mut pending_decorates: Vec<(String, String, String, String)> = Vec::new();

    for entry in &report.entries {
        let rel = entry.rel_path.to_string_lossy().into_owned();
        let bytes = std::fs::read(&entry.abs_path)
            .with_context(|| format!("read {}", entry.abs_path.display()))?;

        // PR6 — File operational metadata (S-005 / AS-013).
        let sha256 = blake3::hash(&bytes); // 32-byte BLAKE3
        let modified_at_ns = std::fs::metadata(&entry.abs_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let loc = bytes.iter().filter(|&&b| b == b'\n').count() as i64;
        let is_generated = is_generated_path(&rel);
        let is_vendored = is_vendored_path(&rel);

        file_rows.push(FileRow {
            path: rel.clone(),
            lang: entry.lang.as_str().to_string(),
            size: entry.size,
            sha256: *sha256.as_bytes(),
            modified_at_ns,
            loc,
            is_generated,
            is_vendored,
        });

        let (tree, symbols, spec) = match parse_file_tree(&rel, entry.lang, &bytes, &cfg, &pool) {
            ParseTreeOutcome::Ok {
                tree,
                symbols,
                spec,
                ..
            } => (tree, symbols, spec),
            ParseTreeOutcome::Skipped => continue,
        };
        let root = tree.root_node();

        for sym in symbols {
            // is_safe_name bounds symbol names to the identifier charset we
            // can safely round-trip through CSV bulk-load + Cypher equality.
            // Pathological names (observed: typescript-eslint rule metadata
            // containing CSS selectors like `VariableDeclarator[init.type=
            // 'ThisExpression'], ...`) are dropped at the parse boundary so
            // no downstream edge (DEFINES/CALLS/REFERENCES/EXTENDS) ever
            // references an unresolvable id. Matches the guard already on
            // call / ref extraction below.
            if !is_safe_name(&sym.name) {
                continue;
            }
            let id = format!("{}::{}:{}", rel, sym.name, sym.line);
            if !symbol_ids_seen.insert(id.clone()) {
                continue;
            }
            symbol_by_file_name
                .entry((rel.clone(), sym.name.clone()))
                .or_insert_with(|| id.clone());
            // S-005a D5: ParsedSymbol.enclosing is now typed (`EnclosingScope`).
            // KG-9 CONTAINS edges only follow class-like enclosure (preserving
            // pre-D5 semantics where enclosing_class meant any OOP container).
            // Module/Namespace/ExtendedType variants are NOT emitted as
            // CONTAINS edges — different semantic relationship.
            if let Some(ga_parser::EnclosingScope::Class(cls)) = sym.enclosing.as_ref() {
                if is_safe_name(cls) && cls != &sym.name {
                    pending_contains.push((id.clone(), rel.clone(), cls.clone()));
                }
            }
            // v1.3 PR3 — denormalize SymbolAttribute → bool cols (AS-010).
            // Suspend / Const / Annotation / Decorator / ExtendedReceiver
            // intentionally NOT mapped: they don't share semantics with the
            // 4 wired bools. Decorator presence is captured indirectly via
            // `is_generated` (synthetic symbols carry confidence < 1.0).
            let mut is_async = false;
            let mut is_override = false;
            let mut is_static = false;
            let mut is_test_marker = false;
            for attr in &sym.attributes {
                match attr {
                    ga_parser::SymbolAttribute::Async => is_async = true,
                    ga_parser::SymbolAttribute::Override => is_override = true,
                    ga_parser::SymbolAttribute::Static => is_static = true,
                    ga_parser::SymbolAttribute::TestMarker => is_test_marker = true,
                    // PR8 — capture decorator/annotation names for DECORATES
                    // edge resolution. Both kinds emit Symbol→Symbol edges
                    // when the named target resolves in-repo.
                    ga_parser::SymbolAttribute::Decorator { name, args } => {
                        pending_decorates.push((
                            id.clone(),
                            rel.clone(),
                            name.clone(),
                            args.clone(),
                        ));
                    }
                    ga_parser::SymbolAttribute::Annotation(name) => {
                        // Annotations don't carry arg-text in v1.3 (Java/C#
                        // attribute_args extraction is per-lang work for
                        // future PR).
                        pending_decorates.push((
                            id.clone(),
                            rel.clone(),
                            name.clone(),
                            String::new(),
                        ));
                    }
                    _ => {}
                }
            }
            // confidence < 1.0 ⇒ synthetic / metaprogrammed symbol
            // (Ruby define_method, Python setattr, JS Object.defineProperty).
            // Per AS-010 these are `is_generated == true`.
            let confidence = sym.confidence as f64;
            let is_generated = confidence < 1.0;
            // v1.3 PR4 — qualified_name = `{rel_path}::{lang_specific_chain}`.
            // Per-lang chain via `LanguageSpec::format_qualified_name`.
            // Dedup pass below resolves AS-005/AS-006 collisions.
            let qn_chain = spec.format_qualified_name(&sym.name, sym.enclosing.as_ref());
            let qualified_name = format!("{rel}::{qn_chain}");
            symbol_rows.push(SymbolRow {
                id: id.clone(),
                name: sym.name,
                file: rel.clone(),
                kind: symbol_kind_str(sym.kind),
                line: sym.line as i64,
                // Schema v3: span end. Synthetic / parser-degraded paths
                // surface line_end == 0 → coerce to `line` so span = 1.
                line_end: if sym.line_end == 0 {
                    sym.line as i64
                } else {
                    sym.line_end as i64
                },
                qualified_name,
                arity: sym.arity.unwrap_or(-1), // Tools-C2 unknown sentinel
                return_type: sym.return_type.unwrap_or_default(), // Tools-C2 empty sentinel
                modifiers: sym.modifiers,
                params: sym.params,
                is_async,
                is_override,
                is_abstract: false, // PR5 — needs per-lang abstract detection
                is_static,
                is_test_marker,
                is_generated,
                confidence,
                // v1.4 H1 fix — set during post-walker resolver pass below
                // (clear_existing_edges + before COPY). Tools-C12 preserved.
                has_unresolved_override: false,
            });
            defines_rows.push((rel.clone(), id));
        }

        // Extract raw call sites — resolution happens after all files parse.
        {
            let calls = extract_calls_from_tree(spec, root, &bytes);
            for c in calls {
                if let Some(enclosing) = c.enclosing_symbol {
                    // Real OSS code occasionally coaxes the call extractor
                    // into returning a whole sub-expression as the callee
                    // name (e.g. `getattr(obj, attr_var)` seen in django).
                    // Those names contain commas / spaces / newlines and
                    // break both CSV bulk-load and downstream Cypher
                    // equality; drop them at the boundary so indexing of
                    // real repos stays robust.
                    if !is_safe_name(&enclosing) || !is_safe_name(&c.callee_name) {
                        continue;
                    }
                    pending_calls.push(PendingCall {
                        file: rel.clone(),
                        caller_name: enclosing,
                        callee_name: c.callee_name,
                        call_site_line: c.call_site_line,
                    });
                }
            }
        }

        // Foundation-C15 — extract value-reference sites. Resolved alongside
        // calls in the second pass (same caller/target lookup model).
        {
            let refs = extract_references_from_tree(spec, entry.lang, root, &bytes);
            for r in refs {
                if !is_safe_name(&r.target_name) {
                    continue;
                }
                let enclosing = r.enclosing_symbol.filter(|s| is_safe_name(s));
                pending_refs.push(PendingRef {
                    file: rel.clone(),
                    caller_name: enclosing,
                    target_name: r.target_name,
                    ref_site_line: r.ref_site_line,
                    ref_kind: r.ref_kind.as_str().to_string(),
                });
            }
        }

        // Extract class inheritance sites — resolved once all symbols known.
        {
            let exts = extract_extends_from_tree(spec, root, &bytes);
            for e in exts {
                pending_extends.push(PendingExtends {
                    file: rel.clone(),
                    class_name: e.class_name,
                    class_line: e.class_line,
                    base_name: e.base_name,
                });
            }
        }

        // Extract import sites — resolve to File nodes in the second pass.
        {
            let imps = extract_imports_from_tree(spec, root, &bytes);
            for i in imps {
                pending_imports.push(PendingImport {
                    src_file: rel.clone(),
                    src_lang: entry.lang,
                    target_path: i.target_path,
                    import_line: i.import_line,
                    imported_names: i.imported_names,
                    imported_aliases: i.imported_aliases,
                    is_re_export: i.is_re_export,
                    type_only_names: i.type_only_names,
                });
            }
        }
    }

    // Build repo-wide name index once — consumed by CALLS (cross-file
    // fallback, Phase A of the cross-file-calls-resolution fix) AND by
    // REFERENCES (same purpose, already in place pre-fix). Only non-external
    // defs count — external placeholders must not resolve other calls into
    // themselves.
    let mut symbol_by_name: HashMap<String, String> = HashMap::new();
    for s in &symbol_rows {
        if s.kind != "external" {
            symbol_by_name
                .entry(s.name.clone())
                .or_insert_with(|| s.id.clone());
        }
    }

    // Hoisted: file_paths used for both import resolution (below) and the
    // downstream IMPORTS edge emission (further below, was line 264).
    let file_paths: HashSet<String> = file_rows.iter().map(|f| f.path.clone()).collect();

    // Phase B — per-file import map. Extracted to phase_b.rs (M-2 refactor)
    // to bound indexer.rs size as v1.1 adds resolver complexity. See
    // `crate::phase_b::build_import_map` for full description.
    let import_map = crate::phase_b::build_import_map(&pending_imports, &file_paths);

    // Resolve pending calls to (caller_id, callee_id).
    // Priority (Foundation-C16):
    //   1. Same-file symbol (unambiguous local scope)
    //   2. Import-map — caller's own `from X import Y` disambiguates when
    //      Y is defined in multiple files (Phase B, this commit)
    //   3. Repo-wide single-def fallback — first-match wins per
    //      symbol_by_name (matches REFERENCES policy at line 225-233;
    //      Phase A of the cross-file-calls-resolution fix)
    //   4. Synthetic `__external__::<name>` placeholder — stdlib /
    //      third-party / truly unresolved. Surfaced to LLMs via
    //      `external: true` on CalleeEntry.
    //
    // See docs/investigate/cross-file-calls-resolution-2026-04-22.md.
    // PR9a — calls_rows tuple includes `is_heuristic` flag for variant routing.
    let mut calls_rows: Vec<(String, String, i64, bool)> = Vec::new();
    let mut calls_seen: HashSet<(String, String, i64)> = HashSet::new();
    let mut external_ids: HashMap<String, String> = HashMap::new();
    for pc in &pending_calls {
        let caller_id = match symbol_by_file_name.get(&(pc.file.clone(), pc.caller_name.clone())) {
            Some(id) => id.clone(),
            None => continue, // caller itself unresolved — drop edge
        };
        // Step 2 — import-map lookup. `import_map` maps
        // (caller_file, local_name) → (target_file, original_name).
        // For aliased imports (infra:S-002 AS-005), original differs from
        // local, so the symbol_by_file_name lookup uses the original name
        // in the target file.
        let import_hit = import_map
            .get(&(pc.file.clone(), pc.callee_name.clone()))
            .and_then(|(dst, original)| {
                symbol_by_file_name
                    .get(&(dst.clone(), original.clone()))
                    .cloned()
            });
        // Resolution order: same-file → import-map → repo-wide name.
        // CALLS keeps single-target resolution. Fan-out in CALLS path
        // over-attributes (function names like `New` in 50 modules
        // resolve every `New(...)` call to all 50, killing dead_code
        // precision — gin audit: 0.718 → 0.500 with fan-out).
        // REFERENCES path uses fan-out (see below) because type names
        // are typically unique and Go build-tag duplicates need it.
        // PR9a — track resolution tier. Tier-3 (repo-wide single-def
        // fallback) is "heuristic": cross-file name match without
        // import-map evidence. Heuristic edges write to BOTH catch-all
        // CALLS and CALLS_HEURISTIC variant per Tools-C7 strict-union.
        let (callee_id, is_heuristic) =
            match symbol_by_file_name.get(&(pc.file.clone(), pc.callee_name.clone())) {
                Some(id) => (id.clone(), false), // tier 1 — same-file
                None => match import_hit {
                    Some(id) => (id, false), // tier 2 — import-resolved
                    None => match symbol_by_name.get(&pc.callee_name) {
                        Some(id) => (id.clone(), true), // tier 3 — heuristic
                        None => (
                            external_ids
                                .entry(pc.callee_name.clone())
                                .or_insert_with(|| format!("__external__::{}", pc.callee_name))
                                .clone(),
                            false, // tier 4 — external (not heuristic; truly unresolved)
                        ),
                    },
                },
            };
        // Self-loop guard intentionally removed: recursive methods and same-name
        // dispatch (Go `Engine.addRoute` calling `node.addRoute`; Rust trait
        // impl calling another impl of the same method) are treated as callers
        // by HdAst's per-file resolution. Dropping self-loops creates FPs
        // where engine says dead but HdAst says alive because it counts the
        // recursive/same-name call as evidence of liveness.
        let pair = (
            caller_id.clone(),
            callee_id.clone(),
            pc.call_site_line as i64,
        );
        if !calls_seen.insert(pair) {
            continue;
        }
        calls_rows.push((caller_id, callee_id, pc.call_site_line as i64, is_heuristic));
    }

    // KG-1 — TESTED_BY derivation from resolved CALLS. Rule: for every
    // caller→callee pair where caller.file matches a test convention AND
    // callee.file does NOT (callee is production code) AND callee is
    // non-external, emit a TESTED_BY edge pointing FROM the production
    // callee TO the test caller — matches the query shape at
    // `impact/affected_tests.rs:40` (`prod -[TESTED_BY]-> test`).
    //
    // Schema + query were wired in M2; only the emission path was missing
    // (see graphatlas-tools.md Known Shipped Gaps KG-1). Convention is the
    // same `common::is_test_path` used by the affected_tests convention
    // matcher so both signals agree on what counts as a test file.
    let file_by_id: HashMap<&str, &str> = symbol_rows
        .iter()
        .map(|s| (s.id.as_str(), s.file.as_str()))
        .collect();
    let mut tested_by_rows: Vec<(String, String)> = Vec::new();
    let mut tested_by_seen: HashSet<(String, String)> = HashSet::new();
    for (caller_id, callee_id, _line, _is_heuristic) in &calls_rows {
        if callee_id.starts_with("__external__::") {
            continue; // stdlib / third-party — not a production symbol
        }
        let Some(caller_file) = file_by_id.get(caller_id.as_str()) else {
            continue;
        };
        let Some(callee_file) = file_by_id.get(callee_id.as_str()) else {
            continue;
        };
        if !common::is_test_path(caller_file) {
            continue; // caller is not a test — not a TESTED_BY edge
        }
        if common::is_test_path(callee_file) {
            continue; // callee is a test helper — skip test→test chains
        }
        let pair = (callee_id.clone(), caller_id.clone());
        if !tested_by_seen.insert(pair.clone()) {
            continue;
        }
        tested_by_rows.push(pair);
    }

    // KG-9 — resolve CONTAINS edges (class_symbol → member_symbol). Same
    // file lookup: member's `enclosing_class` name must resolve to a class
    // symbol declared in the same file (classes can't span files in the
    // 5 langs we support today). Emission shape mirrors rust-poc:1935-1962
    // — this enables the sibling-method reverse-forward traversal
    // documented at rust-poc:2217 for OOP blast radius once BFS is
    // extended (KG-9 Action 2, separate session).
    let mut contains_rows: Vec<(String, String)> = Vec::new();
    let mut contains_seen: HashSet<(String, String)> = HashSet::new();
    for (member_id, file, class_name) in &pending_contains {
        let Some(class_id) = symbol_by_file_name.get(&(file.clone(), class_name.clone())) else {
            continue; // enclosing class name didn't match a symbol in the same file
        };
        if class_id == member_id {
            continue; // self-reference guard (class named same as member)
        }
        let pair = (class_id.clone(), member_id.clone());
        if !contains_seen.insert(pair.clone()) {
            continue;
        }
        contains_rows.push(pair);
    }

    // Foundation-C15 — resolve REFERENCES edges.
    // Same model as CALLS: caller name in same file → caller id; target
    // name in same file → target id, else lookup across files (prefer
    // same-file, fallback first matching symbol). Skip if either endpoint
    // unresolved — REFERENCES to external names drop silently (policy).
    let mut refs_rows: Vec<(String, String, i64, String)> = Vec::new();
    let mut refs_seen: HashSet<(String, String, i64)> = HashSet::new();
    for pr in &pending_refs {
        let Some(caller_name) = &pr.caller_name else {
            // TypePosition refs at module scope (impl Trait, static/const type
            // annotations) have no enclosing function. Emit a MODULE_TYPED edge
            // (File → Symbol) so dead_code doesn't flag the target as 0-in-degree.
            if pr.ref_kind == "type_position" {
                let target_id =
                    match symbol_by_file_name.get(&(pr.file.clone(), pr.target_name.clone())) {
                        Some(id) => id.clone(),
                        None => match symbol_by_name.get(&pr.target_name) {
                            Some(id) => id.clone(),
                            None => continue,
                        },
                    };
                module_typed_rows.push((pr.file.clone(), target_id));
            }
            continue;
        };
        let caller_id = match symbol_by_file_name.get(&(pr.file.clone(), caller_name.clone())) {
            Some(id) => id.clone(),
            None => continue,
        };
        // Target: same-file first, else repo-wide.
        let target_id = match symbol_by_file_name.get(&(pr.file.clone(), pr.target_name.clone())) {
            Some(id) => id.clone(),
            None => match symbol_by_name.get(&pr.target_name) {
                Some(id) => id.clone(),
                None => continue, // external / unknown — drop
            },
        };
        if caller_id == target_id {
            continue;
        }
        let key = (
            caller_id.clone(),
            target_id.clone(),
            pr.ref_site_line as i64,
        );
        if !refs_seen.insert(key) {
            continue;
        }
        refs_rows.push((
            caller_id,
            target_id,
            pr.ref_site_line as i64,
            pr.ref_kind.clone(),
        ));
    }

    // Resolve pending imports to CSV rows. Tools-C12 scope.
    let imports_rows: Vec<ImportRow> = resolve_pending_imports(&pending_imports, &file_paths);

    // PR8 / S-006 AS-016 — resolve DECORATES edges. For each pending
    // (decorated_id, decorated_file, decorator_name) tuple, look up
    // `decorator_name` first in same-file, then by last-segment fallback
    // for dotted names (`@helper.cached` → "cached"), then repo-wide. If
    // resolved, emit DECORATES(decorator_id → decorated_id). Unresolved
    // (most stdlib decorators like `@functools.lru_cache`) drop silently —
    // counter exposed via Tools-C4 stats in a future PR.
    let mut decorates_rows: Vec<(String, String, String)> = Vec::new();
    let mut decorates_seen: HashSet<(String, String)> = HashSet::new();
    let mut unresolved_decorators: usize = 0;
    for (decorated_id, decorated_file, decorator_name, decorator_args) in &pending_decorates {
        // Try as-is in same file
        let mut hit = symbol_by_file_name
            .get(&(decorated_file.clone(), decorator_name.clone()))
            .cloned();
        // Fallback: last segment of dotted name (`app.route` → `route`)
        if hit.is_none() {
            if let Some(last) = decorator_name.rsplit('.').next() {
                if last != decorator_name {
                    hit = symbol_by_file_name
                        .get(&(decorated_file.clone(), last.to_string()))
                        .cloned()
                        .or_else(|| symbol_by_name.get(last).cloned());
                }
            }
        }
        // Fallback: repo-wide name
        if hit.is_none() {
            hit = symbol_by_name.get(decorator_name).cloned();
        }
        let Some(decorator_id) = hit else {
            unresolved_decorators += 1;
            continue;
        };
        if decorator_id == *decorated_id {
            // Self-decoration is meaningless — drop.
            continue;
        }
        let key = (decorator_id.clone(), decorated_id.clone());
        if !decorates_seen.insert(key) {
            continue;
        }
        decorates_rows.push((decorator_id, decorated_id.clone(), decorator_args.clone()));
    }
    // unresolved_decorators surfaced through IndexStats below (Tools-C4).

    // PR7 / S-006 AS-014 — IMPORTS_NAMED edge resolution. Walk PendingImport
    // entries; for each that resolves to a target file in the indexed set,
    // look up each imported name as a Symbol in that file and emit a
    // (src_file, target_symbol_id, import_line, alias) tuple.
    //
    // Multiplicity per `(import_line, alias, target_symbol_id)` per spec.
    // External / unresolved names drop silently (Tools-C12). The legacy
    // IMPORTS (File→File) edge is also emitted (existing path) per Tools-C7
    // strict-union catch-all.
    // v1.4 S-002 — IMPORTS_NAMED row payload extended with is_type_only
    // (5th column). Dedup key remains (src, target_id, line, alias) — same
    // statement at the same line can never carry both type and value
    // semantics for the same name.
    let mut imports_named_rows: Vec<(String, String, i64, String, bool)> = Vec::new();
    let mut imports_named_seen: HashSet<(String, String, i64, String)> = HashSet::new();
    // Tools-C4 — count names that resolved to a target file but whose
    // symbol wasn't defined there. Distinct from "external import" (file
    // not in repo, where we don't even attempt name resolution).
    let mut unresolved_imports_count: usize = 0;
    for pi in &pending_imports {
        let resolved_dst = crate::import_resolve::resolve_import_path(
            &pi.target_path,
            pi.src_lang,
            &pi.src_file,
            &file_paths,
        );
        // B1 — langs without a path-resolver (Rust today) fall back to
        // repo-wide name lookup via `symbol_by_name`. `use foo::Bar` →
        // resolve "Bar" to its unique Symbol regardless of module path.
        // Name collisions silently keep the first match (matches Tools-C7
        // policy already applied at `symbol_by_name` build site).
        let lang_uses_name_fallback = matches!(pi.src_lang, ga_core::Lang::Rust);

        // Resolve a single (target_name) to a Symbol id.
        let resolve_one = |name: &str| -> Option<String> {
            if let Some(dst) = &resolved_dst {
                symbol_by_file_name
                    .get(&(dst.clone(), name.to_string()))
                    .cloned()
            } else if lang_uses_name_fallback {
                symbol_by_name.get(name).cloned()
            } else {
                None
            }
        };

        // If neither a resolved path nor a name-fallback applies, skip.
        if resolved_dst.is_none() && !lang_uses_name_fallback {
            continue;
        }
        if let Some(dst) = &resolved_dst {
            if dst == &pi.src_file {
                continue;
            }
        }
        // v1.4 S-002 — local-name → is_type_only fast lookup (TS-only;
        // empty for other langs).
        let type_only_set: HashSet<&String> = pi.type_only_names.iter().collect();
        // Aliased imports first — `(local, original)` pairs. Look up by
        // `original`; alias = `local`.
        let aliased_locals: HashSet<String> =
            pi.imported_aliases.iter().map(|(l, _)| l.clone()).collect();
        for (local, original) in &pi.imported_aliases {
            let Some(target_id) = resolve_one(original) else {
                unresolved_imports_count += 1; // Tools-C4
                continue;
            };
            // Self-edge guard: target Symbol's file == source file.
            if let Some(tgt_file) = file_by_id.get(target_id.as_str()) {
                if *tgt_file == pi.src_file {
                    continue;
                }
            }
            let key = (
                pi.src_file.clone(),
                target_id.clone(),
                pi.import_line as i64,
                local.clone(),
            );
            if imports_named_seen.insert(key.clone()) {
                let is_type_only = type_only_set.contains(local);
                let (s, t, l, a) = key;
                imports_named_rows.push((s, t, l, a, is_type_only));
            }
        }
        // Non-aliased names — alias = ''.
        for name in &pi.imported_names {
            // Skip names that already had an alias entry (avoid double-emit).
            if aliased_locals.contains(name) {
                continue;
            }
            let Some(target_id) = resolve_one(name) else {
                unresolved_imports_count += 1; // Tools-C4
                continue;
            };
            if let Some(tgt_file) = file_by_id.get(target_id.as_str()) {
                if *tgt_file == pi.src_file {
                    continue;
                }
            }
            let key = (
                pi.src_file.clone(),
                target_id.clone(),
                pi.import_line as i64,
                String::new(),
            );
            if imports_named_seen.insert(key.clone()) {
                let is_type_only = type_only_set.contains(name);
                let (s, t, l, a) = key;
                imports_named_rows.push((s, t, l, a, is_type_only));
            }
        }
    }

    // Resolve EXTENDS: (deriving_class_id, base_class_id). The deriving
    // class is always in-repo (we parsed its file). The base is best-effort:
    // we look up by short name across ALL files and take the first
    // (or skip if 0 — external base class). Python inheritance can
    // reference bases from other modules via imports; we don't follow
    // imports here. An indexed repo with both A and class B(A) in scope
    // emits the edge; cross-repo bases silently drop.
    let mut extends_rows: Vec<(String, String)> = Vec::new();
    let mut extends_seen: HashSet<(String, String)> = HashSet::new();
    // PR9b — IMPLEMENTS subset of EXTENDS. Strict-union per Tools-C7:
    // EVERY IMPLEMENTS row also exists in EXTENDS catch-all.
    let mut implements_rows: Vec<(String, String)> = Vec::new();
    let mut implements_seen: HashSet<(String, String)> = HashSet::new();
    // name → first-seen symbol id (ignores external placeholders).
    // Broadened from class-only to include struct/trait/interface/enum so
    // Rust `impl Trait for Struct` + TS `class X extends Y` both resolve.
    // External placeholders keep kind="external" → excluded by design.
    let mut class_by_name: HashMap<String, String> = HashMap::new();
    // PR9b — id → kind map for Class-vs-Interface-vs-Trait disambiguation.
    let mut id_to_kind: HashMap<String, String> = HashMap::new();
    for s in &symbol_rows {
        id_to_kind.insert(s.id.clone(), s.kind.clone());
        if matches!(
            s.kind.as_str(),
            "class" | "struct" | "trait" | "interface" | "enum"
        ) {
            class_by_name
                .entry(s.name.clone())
                .or_insert_with(|| s.id.clone());
        }
    }
    for pe in &pending_extends {
        // src lookup: prefer same-file (TS/Python class definition lives with
        // its inheritance declaration) → fall back to repo-wide for Rust,
        // where `struct Bar` and `impl Trait for Bar` often split files.
        let src_id = match symbol_by_file_name.get(&(pe.file.clone(), pe.class_name.clone())) {
            Some(id) => id.clone(),
            None => match class_by_name.get(&pe.class_name) {
                Some(id) => id.clone(),
                None => continue, // class truly unknown — drop
            },
        };
        let dst_id = match class_by_name.get(&pe.base_name) {
            Some(id) => id.clone(),
            None => continue, // base not in repo — drop silently
        };
        if src_id == dst_id {
            continue;
        }
        let pair = (src_id.clone(), dst_id.clone());
        if !extends_seen.insert(pair.clone()) {
            continue;
        }
        // PR9b — also emit IMPLEMENTS when target is an interface or trait.
        // Universal-truth: type system distinguishes "is-a class hierarchy"
        // from "satisfies interface contract" via the target's kind.
        // Java `class C implements I` → I.kind == "interface" ⇒ IMPLEMENTS.
        // Rust `impl Display for Foo` → Display.kind == "trait" ⇒ IMPLEMENTS.
        // Java `class B extends A` → A.kind == "class" ⇒ EXTENDS only.
        if let Some(target_kind) = id_to_kind.get(&dst_id) {
            if matches!(target_kind.as_str(), "interface" | "trait") {
                let impl_pair = (src_id.clone(), dst_id.clone());
                if implements_seen.insert(impl_pair.clone()) {
                    implements_rows.push(impl_pair);
                }
            }
        }
        extends_rows.push((src_id, dst_id));
    }

    // v1.4 (S-001a) — OVERRIDES resolver. For each Symbol with `is_override
    // = true`, find its enclosing class via reverse `contains_rows`, find
    // the parent class via `extends_rows` (single-step / immediate parent
    // per Q1 clarification), look up the parent method by name in that
    // parent class. If found AND not self → emit OVERRIDES(child, parent).
    // If parent class or parent method missing → set
    // `has_unresolved_override = true` on the child Symbol (Tools-C12 no
    // synthetic edge; H1 fix — `ga_dead_code` rescues external-parent
    // overrides via the flag). Self-override → skip + counter (Tools-C18
    // class-level witness AT-009 invariant). The resolver runs AFTER
    // walker collection completes (Tools-C19 race guard); whether
    // implemented in-memory pre-COPY or as a post-COPY DB phase is
    // implementation choice — this build uses in-memory pre-COPY because
    // walker is single-threaded and `extends_rows` + `symbol_rows` are
    // fully populated before this point. Tools-C19 spec wording "post-
    // Symbol-COPY" is over-specified; in-memory pre-COPY satisfies the
    // race-free guarantee identically.
    let mut overrides_rows: Vec<(String, String)> = Vec::new();
    let mut overrides_seen: HashSet<(String, String)> = HashSet::new();
    let mut unresolved_overrides_count: usize = 0;
    let mut self_override_skip_count: usize = 0;
    {
        // Reverse contains_rows: child_method_id → enclosing_class_id.
        // Tools-C7 invariant: single class container per method.
        let mut method_to_class: HashMap<String, String> = HashMap::new();
        for (class_id, member_id) in &contains_rows {
            method_to_class
                .entry(member_id.clone())
                .or_insert_with(|| class_id.clone());
        }
        // child_class → parent_class via extends_rows. Single-step (Java/
        // Kotlin/C# single inheritance; first-seen parent if multiple).
        let mut class_to_parent: HashMap<String, String> = HashMap::new();
        for (sub_class_id, par_class_id) in &extends_rows {
            class_to_parent
                .entry(sub_class_id.clone())
                .or_insert_with(|| par_class_id.clone());
        }
        // (class_id, method_name) → method_symbol_id.
        let mut method_in_class: HashMap<(String, String), String> = HashMap::new();
        for s in &symbol_rows {
            if let Some(cls_id) = method_to_class.get(&s.id) {
                method_in_class
                    .entry((cls_id.clone(), s.name.clone()))
                    .or_insert_with(|| s.id.clone());
            }
        }

        // Resolution pass — collect indices that need has_unresolved_override
        // flag set; mutate symbol_rows in a second pass to avoid borrow
        // conflicts.
        let mut needs_unresolved: Vec<usize> = Vec::new();
        for (idx, s) in symbol_rows.iter().enumerate() {
            if !s.is_override {
                continue;
            }
            let child_id = &s.id;
            // Step 1: enclosing class (Tools-C18 sub_class for the witness).
            let class_id = match method_to_class.get(child_id) {
                Some(c) => c,
                None => {
                    needs_unresolved.push(idx);
                    continue;
                }
            };
            // Step 2: parent class (single-step per Q1).
            let par_class_id = match class_to_parent.get(class_id) {
                Some(p) => p,
                None => {
                    needs_unresolved.push(idx);
                    continue;
                }
            };
            // Step 3: parent method by name within parent class.
            let par_method_id = match method_in_class.get(&(par_class_id.clone(), s.name.clone())) {
                Some(m) => m,
                None => {
                    needs_unresolved.push(idx);
                    continue;
                }
            };
            // Step 4: self-override guard (AS-003 / AT-009).
            if par_method_id == child_id {
                self_override_skip_count += 1;
                continue;
            }
            let pair = (child_id.clone(), par_method_id.clone());
            if overrides_seen.insert(pair.clone()) {
                overrides_rows.push(pair);
            }
        }
        for idx in needs_unresolved {
            symbol_rows[idx].has_unresolved_override = true;
            unresolved_overrides_count += 1;
        }
    }

    // Emit synthetic external Symbol rows so the CALLS FK holds. No DEFINES
    // edge for these — they live outside the repo's File set.
    for (name, id) in &external_ids {
        if symbol_ids_seen.insert(id.clone()) {
            symbol_rows.push(SymbolRow {
                id: id.clone(),
                name: name.clone(),
                file: "<external>".to_string(),
                kind: "external".to_string(),
                line: 0,
                line_end: 0,
                // External placeholders carry empty qualified_name — they
                // don't represent in-repo definitions, so AT-001 audit
                // explicitly excludes `kind = 'external'` rows.
                qualified_name: String::new(),
                arity: -1,                  // Tools-C2 unknown sentinel
                return_type: String::new(), // Tools-C2 empty sentinel
                modifiers: Vec::new(),
                params: Vec::new(),
                is_async: false,
                is_override: false,
                is_abstract: false,
                is_static: false,
                is_test_marker: false,
                is_generated: false,
                confidence: 1.0,
                has_unresolved_override: false,
            });
        }
    }

    // Drop duplicate File rows too (walker shouldn't emit duplicates, but
    // defence-in-depth — matches the S-003 cache-dir story).
    let mut seen_files = HashSet::<String>::new();
    file_rows.retain(|f| seen_files.insert(f.path.clone()));

    // v1.3 PR4 / AS-006 — qualified_name dedup pass. lbug 0.16.1 has no
    // UNIQUE non-PK constraint (Tools-C5). When two SymbolRows hit the same
    // qualified_name (Rust macro-expanded same-name symbols, overloads, or
    // rare per-lang spec gaps), append `#dup<N>` suffix in walker emission
    // order. First occurrence keeps the bare qn; second → `#dup1`, third →
    // `#dup2`, etc. Order-invariant per Tools-C5: walker emission is
    // deterministic (tree-sitter parse order), so two indexes of the same
    // commit produce identical suffixed qns. External rows excluded
    // (qualified_name == ''). Collision counter exposed via stats — Tools-C4.
    let mut qn_counts: HashMap<String, usize> = HashMap::new();
    let mut qn_collisions: usize = 0;
    for s in symbol_rows.iter_mut() {
        if s.qualified_name.is_empty() {
            continue; // external placeholder — skip dedup
        }
        let count = qn_counts.entry(s.qualified_name.clone()).or_insert(0);
        if *count > 0 {
            qn_collisions += 1;
            let suffixed = format!("{}#dup{}", s.qualified_name, *count);
            // WARN log per AS-006 Then clause (Tools-C5 reference).
            eprintln!(
                "WARN: qualified_name collision {} at {} (kept first); see v1.3-Tools-C5",
                s.qualified_name, s.id
            );
            s.qualified_name = suffixed;
        }
        *count += 1;
    }
    // qn_collisions surfaced through IndexStats below (Tools-C4).

    // Bulk-write via CSV + COPY. Pattern ported from
    // spike/ldb-spike/src/main.rs::write_batch + rust-poc/src/main.rs:1606.
    let tmp = tempfile::tempdir().context("csv tempdir")?;

    let files_csv = tmp.path().join("files.csv");
    {
        let w = std::fs::File::create(&files_csv)?;
        let mut buf = BufWriter::new(w);
        for f in &file_rows {
            // PR6 — 8 cols: path, lang, size + sha256 (BLOB hex literal),
            // modified_at (TIMESTAMP literal), loc, is_generated, is_vendored.
            // lbug BLOB CSV format: hex-encoded with no prefix (verified by
            // empirical test below — if syntax wrong, all File rows drop).
            // TIMESTAMP CSV format: ISO 8601 string (UTC).
            let sha_hex = blob_csv_literal(&f.sha256);
            let secs = f.modified_at_ns / 1_000_000_000;
            let nanos_part = (f.modified_at_ns % 1_000_000_000) as u32;
            // Format as ISO 8601 — lbug parses `YYYY-MM-DD HH:MM:SS` (UTC).
            let ts = format_iso8601_utc(secs, nanos_part);
            writeln!(
                buf,
                "{},{},{},{},{},{},{},{}",
                f.path, f.lang, f.size, sha_hex, ts, f.loc, f.is_generated, f.is_vendored
            )?;
        }
        buf.flush()?;
    }

    let syms_csv = tmp.path().join("symbols.csv");
    {
        let w = std::fs::File::create(&syms_csv)?;
        let mut buf = BufWriter::new(w);
        for s in &symbol_rows {
            // v1.3 PR4 — 14 cols (v3 6 + qualified_name + 7 PR3 attr/confidence).
            // return_type / arity / doc_summary still omitted → take DDL
            // DEFAULT (PR5 will populate). Tools-C13: only safe DEFAULT-omit
            // types (STRING / INT64 / BOOL / DOUBLE) omitted. Composite
            // STRUCT[] / LIST<STRING> stay deferred.
            //
            // qualified_name is positional col 7. Quoted to survive CSV parser
            // when chain contains separators (`::`, `.`, `#`) — though our
            // separators are CSV-safe, defence-in-depth via lbug's `\"...\"`
            // string literal escaping. Replace any `"` in the qn (rare) with
            // `""` per CSV RFC 4180.
            // PR15 fix — strip \n / \r from outer-quoted CSV fields. lbug
            // parallel CSV reader rejects "Quoted newlines" (radash fixture
            // had a multi-line return_type from line-continued generics).
            // CSV `"` escape unchanged.
            let qn_escaped = s
                .qualified_name
                .replace(['\n', '\r'], " ")
                .replace('"', "\"\"");
            let rt_escaped = s
                .return_type
                .replace(['\n', '\r'], " ")
                .replace('"', "\"\"");
            // PR5c2b — lbug LIST<STRING> CSV: `"[a,b,c]"` quoted bracket.
            // STRUCT[] CSV: `"[{name: x, type: i32, default_value: }]"`. Per
            // spike #1 Variant B. Sanitize struct field values: `,` and `:`
            // are inline-struct delimiters; values containing them (Rust
            // `HashMap<String, i32>`, Python `lambda` defaults) would break
            // parsing. Replace problem chars with space — lossy but universal-
            // truth: type identity preserved (`HashMap<String  i32>` is still
            // the same generic).
            // PR15 fix — strip ALL chars that lbug inline-struct CSV parser
            // treats as delimiters / quote markers. Real-world fixtures hit:
            // - axum `&'static str` (apostrophe breaks STRUCT[] parser)
            // - httpx `default_value: ""` (double-quote)
            // - radash multi-line generics (newline)
            // Lossy but universal-truth: type identity preserved (`&'static str`
            // → `& static str` is still recognizable).
            fn sanitize_struct_field(s: &str) -> String {
                s.replace(
                    [',', ':', '{', '}', '[', ']', '\'', '"', '\\', '\n', '\r'],
                    " ",
                )
                .trim()
                .to_string()
            }
            let mods_csv = if s.modifiers.is_empty() {
                "[]".to_string()
            } else {
                let safe: Vec<String> = s
                    .modifiers
                    .iter()
                    .map(|m| m.replace([',', '[', ']', '\'', '"', '\n', '\r'], " "))
                    .collect();
                format!("[{}]", safe.join(","))
            };
            let params_csv = if s.params.is_empty() {
                "[]".to_string()
            } else {
                let inner: Vec<String> = s
                    .params
                    .iter()
                    .map(|p| {
                        format!(
                            "{{name: {}, type: {}, default_value: {}}}",
                            sanitize_struct_field(&p.name),
                            sanitize_struct_field(&p.type_),
                            sanitize_struct_field(&p.default_value),
                        )
                    })
                    .collect();
                format!("[{}]", inner.join(","))
            };
            writeln!(
                buf,
                "{},{},{},{},{},{},\"{}\",\"{}\",{},{},{},{},{},{},{},{},{},\"{}\",\"{}\"",
                s.id,
                s.name,
                s.file,
                s.kind,
                s.line,
                s.line_end,
                qn_escaped,
                rt_escaped,
                s.arity,
                s.is_async,
                s.is_override,
                s.is_abstract,
                s.is_static,
                s.is_test_marker,
                s.is_generated,
                s.confidence,
                s.has_unresolved_override,
                mods_csv,
                params_csv,
            )?;
        }
        buf.flush()?;
    }

    let defines_csv = tmp.path().join("defines.csv");
    {
        let w = std::fs::File::create(&defines_csv)?;
        let mut buf = BufWriter::new(w);
        for (file, sym_id) in &defines_rows {
            writeln!(buf, "{},{}", file, sym_id)?;
        }
        buf.flush()?;
    }

    let calls_csv = tmp.path().join("calls.csv");
    {
        let w = std::fs::File::create(&calls_csv)?;
        let mut buf = BufWriter::new(w);
        for (caller_id, callee_id, line, _is_heuristic) in &calls_rows {
            writeln!(buf, "{},{},{}", caller_id, callee_id, line)?;
        }
        buf.flush()?;
    }

    // PR9a — CALLS_HEURISTIC subset. Tier-3 repo-wide-fallback edges.
    // Tools-C7 strict-union: every CALLS_HEURISTIC row ALSO exists in CALLS
    // catch-all (the loop above writes every row to CALLS regardless of tier).
    let calls_heuristic_csv = tmp.path().join("calls_heuristic.csv");
    let mut calls_heuristic_count = 0usize;
    {
        let w = std::fs::File::create(&calls_heuristic_csv)?;
        let mut buf = BufWriter::new(w);
        for (caller_id, callee_id, line, is_heuristic) in &calls_rows {
            if !*is_heuristic {
                continue;
            }
            writeln!(buf, "{},{},{}", caller_id, callee_id, line)?;
            calls_heuristic_count += 1;
        }
        buf.flush()?;
    }

    let imports_csv = tmp.path().join("imports.csv");
    {
        let w = std::fs::File::create(&imports_csv)?;
        let mut buf = BufWriter::new(w);
        for (src, dst, line, names, reexp) in &imports_rows {
            writeln!(buf, "{},{},{},{},{}", src, dst, line, names, reexp)?;
        }
        buf.flush()?;
    }

    // PR8 — DECORATES edges. Positional CSV: (decorator_id, decorated_id,
    // decorator_args). Gap 6 — args carry raw paren-stripped source text
    // for Python decorators (e.g. `'/users', methods=['GET']`). Tools-C14
    // sanitization strips CSV-unsafe chars (`,`, `"`, `\n`) so the cell
    // round-trips. Java/C# annotations + Ruby synthetic `define_method`
    // emit empty args.
    let decorates_csv = tmp.path().join("decorates.csv");
    {
        let w = std::fs::File::create(&decorates_csv)?;
        let mut buf = BufWriter::new(w);
        for (dec_id, target_id, args) in &decorates_rows {
            // Tools-C14 sanitize: replace CSV-unsafe chars in args. Outer
            // value is unquoted (no `"` wrapper), so strip `"` + `,` + newlines.
            let safe = args.replace([',', '"', '\n', '\r'], " ").trim().to_string();
            writeln!(buf, "{},{},{}", dec_id, target_id, safe)?;
        }
        buf.flush()?;
    }

    // PR7 — IMPORTS_NAMED edges. lbug REL COPY uses positional CSV:
    // (from_pk, to_pk, ...rel_props in DDL order). DDL props (v1.4):
    // import_line, alias, re_export, re_export_source, is_type_only.
    // Re-export source resolution deferred — emit `''` constant. v1.4
    // S-002 added is_type_only as the final column.
    let imports_named_csv = tmp.path().join("imports_named.csv");
    {
        let w = std::fs::File::create(&imports_named_csv)?;
        let mut buf = BufWriter::new(w);
        for (src, target_id, line, alias, is_type_only) in &imports_named_rows {
            writeln!(
                buf,
                "{},{},{},{},{},{},{}",
                src, target_id, line, alias, false, "", is_type_only
            )?;
        }
        buf.flush()?;
    }

    let extends_csv = tmp.path().join("extends.csv");
    {
        let w = std::fs::File::create(&extends_csv)?;
        let mut buf = BufWriter::new(w);
        for (src, dst) in &extends_rows {
            writeln!(buf, "{},{}", src, dst)?;
        }
        buf.flush()?;
    }

    // PR9b — IMPLEMENTS subset.
    let implements_csv = tmp.path().join("implements.csv");
    {
        let w = std::fs::File::create(&implements_csv)?;
        let mut buf = BufWriter::new(w);
        for (src, dst) in &implements_rows {
            writeln!(buf, "{},{}", src, dst)?;
        }
        buf.flush()?;
    }

    // v1.4 (S-001a) — OVERRIDES Symbol→Symbol edges.
    let overrides_csv = tmp.path().join("overrides.csv");
    {
        let w = std::fs::File::create(&overrides_csv)?;
        let mut buf = BufWriter::new(w);
        for (child, par) in &overrides_rows {
            writeln!(buf, "{},{}", child, par)?;
        }
        buf.flush()?;
    }

    let refs_csv = tmp.path().join("references.csv");
    {
        let w = std::fs::File::create(&refs_csv)?;
        let mut buf = BufWriter::new(w);
        for (caller_id, target_id, line, ref_kind) in &refs_rows {
            writeln!(buf, "{},{},{},{}", caller_id, target_id, line, ref_kind)?;
        }
        buf.flush()?;
    }

    let module_typed_csv = tmp.path().join("module_typed.csv");
    {
        let w = std::fs::File::create(&module_typed_csv)?;
        let mut buf = BufWriter::new(w);
        for (file, target_id) in &module_typed_rows {
            writeln!(buf, "{},{}", file, target_id)?;
        }
        buf.flush()?;
    }

    let tested_by_csv = tmp.path().join("tested_by.csv");
    {
        let w = std::fs::File::create(&tested_by_csv)?;
        let mut buf = BufWriter::new(w);
        for (prod_id, test_id) in &tested_by_rows {
            writeln!(buf, "{},{}", prod_id, test_id)?;
        }
        buf.flush()?;
    }

    let contains_csv = tmp.path().join("contains.csv");
    {
        let w = std::fs::File::create(&contains_csv)?;
        let mut buf = BufWriter::new(w);
        for (class_id, member_id) in &contains_rows {
            writeln!(buf, "{},{}", class_id, member_id)?;
        }
        buf.flush()?;
    }

    let conn = store.connection().map_err(|e| anyhow!("connection: {e}"))?;

    // Clear existing data so reindex is idempotent. v1.4 Tools-C21:
    // iterate ga_index::schema::REL_DELETE_STATEMENTS instead of an
    // inline hand-maintained list — the architectural test
    // `crates/ga-index/tests/rel_delete_parity.rs` enforces that every
    // CREATE REL TABLE in BASE_DDL_STATEMENTS has a matching DELETE here.
    for stmt in ga_index::schema::REL_DELETE_STATEMENTS {
        let _ = conn.query(stmt);
    }
    let _ = conn.query("MATCH (s:Symbol) DETACH DELETE s");
    let _ = conn.query("MATCH (f:File) DETACH DELETE f");

    // v1.3-Tools-C10 — explicit column list aligns positional CSV with v3 cols
    // only. New v4 columns (sha256, modified_at, loc, is_generated, is_vendored
    // on File; qualified_name, params, return_type, modifiers, arity, 6 bools,
    // confidence, doc_summary on Symbol) take their DDL DEFAULTs in PR1 since
    // PR2-PR8 are responsible for populating them via parser/walker work.
    conn.query(&format!(
        "COPY File (path, lang, size, sha256, modified_at, loc, is_generated, is_vendored) \
         FROM '{}' (header=false)",
        files_csv.display()
    ))
    .map_err(|e| anyhow!("COPY File failed: {e}"))?;

    if !symbol_rows.is_empty() {
        conn.query(&format!(
            "COPY Symbol (id, name, file, kind, line, line_end, qualified_name, return_type, arity, \
             is_async, is_override, is_abstract, is_static, is_test_marker, \
             is_generated, confidence, has_unresolved_override, modifiers, params) FROM '{}' (header=false)",
            syms_csv.display()
        ))
        .map_err(|e| anyhow!("COPY Symbol failed: {e}"))?;
    }

    if !defines_rows.is_empty() {
        conn.query(&format!(
            "COPY DEFINES FROM '{}' (header=false)",
            defines_csv.display()
        ))
        .map_err(|e| anyhow!("COPY DEFINES failed: {e}"))?;
    }

    if !calls_rows.is_empty() {
        conn.query(&format!(
            "COPY CALLS FROM '{}' (header=false)",
            calls_csv.display()
        ))
        .map_err(|e| anyhow!("COPY CALLS failed: {e}"))?;
    }

    if calls_heuristic_count > 0 {
        conn.query(&format!(
            "COPY CALLS_HEURISTIC FROM '{}' (header=false)",
            calls_heuristic_csv.display()
        ))
        .map_err(|e| anyhow!("COPY CALLS_HEURISTIC failed: {e}"))?;
    }

    if !imports_rows.is_empty() {
        conn.query(&format!(
            "COPY IMPORTS FROM '{}' (header=false)",
            imports_csv.display()
        ))
        .map_err(|e| anyhow!("COPY IMPORTS failed: {e}"))?;
    }

    if !imports_named_rows.is_empty() {
        conn.query(&format!(
            "COPY IMPORTS_NAMED FROM '{}' (header=false)",
            imports_named_csv.display()
        ))
        .map_err(|e| anyhow!("COPY IMPORTS_NAMED failed: {e}"))?;
    }

    if !decorates_rows.is_empty() {
        conn.query(&format!(
            "COPY DECORATES FROM '{}' (header=false)",
            decorates_csv.display()
        ))
        .map_err(|e| anyhow!("COPY DECORATES failed: {e}"))?;
    }

    if !extends_rows.is_empty() {
        conn.query(&format!(
            "COPY EXTENDS FROM '{}' (header=false)",
            extends_csv.display()
        ))
        .map_err(|e| anyhow!("COPY EXTENDS failed: {e}"))?;
    }

    if !implements_rows.is_empty() {
        conn.query(&format!(
            "COPY IMPLEMENTS FROM '{}' (header=false)",
            implements_csv.display()
        ))
        .map_err(|e| anyhow!("COPY IMPLEMENTS failed: {e}"))?;
    }

    if !overrides_rows.is_empty() {
        conn.query(&format!(
            "COPY OVERRIDES FROM '{}' (header=false)",
            overrides_csv.display()
        ))
        .map_err(|e| anyhow!("COPY OVERRIDES failed: {e}"))?;
    }

    if !refs_rows.is_empty() {
        conn.query(&format!(
            "COPY REFERENCES FROM '{}' (header=false)",
            refs_csv.display()
        ))
        .map_err(|e| anyhow!("COPY REFERENCES failed: {e}"))?;
    }

    if !tested_by_rows.is_empty() {
        conn.query(&format!(
            "COPY TESTED_BY FROM '{}' (header=false)",
            tested_by_csv.display()
        ))
        .map_err(|e| anyhow!("COPY TESTED_BY failed: {e}"))?;
    }

    if !contains_rows.is_empty() {
        conn.query(&format!(
            "COPY CONTAINS FROM '{}' (header=false)",
            contains_csv.display()
        ))
        .map_err(|e| anyhow!("COPY CONTAINS failed: {e}"))?;
    }

    if !module_typed_rows.is_empty() {
        conn.query(&format!(
            "COPY MODULE_TYPED FROM '{}' (header=false)",
            module_typed_csv.display()
        ))
        .map_err(|e| anyhow!("COPY MODULE_TYPED failed: {e}"))?;
    }

    Ok(IndexStats {
        files: file_rows.len(),
        symbols: symbol_rows.len(),
        defines_edges: defines_rows.len(),
        calls_edges: calls_rows.len(),
        imports_edges: imports_rows.len(),
        extends_edges: extends_rows.len(),
        references_edges: refs_rows.len(),
        module_typed_edges: module_typed_rows.len(),
        // Tools-C4 — counters surfaced from PR4/PR7/PR8 dedup + resolution paths.
        qualified_name_collision_count: qn_collisions,
        unresolved_imports_count,
        unresolved_decorators_count: unresolved_decorators,
        // v1.4 S-001a counters (parent-method resolver)
        unresolved_overrides_count,
        self_override_skip_count,
    })
}

struct PendingCall {
    file: String,
    caller_name: String,
    callee_name: String,
    call_site_line: u32,
}

struct PendingExtends {
    file: String,
    class_name: String,
    #[allow(dead_code)] // kept for future source-position resolution
    class_line: u32,
    base_name: String,
}

struct PendingRef {
    file: String,
    caller_name: Option<String>,
    target_name: String,
    ref_site_line: u32,
    ref_kind: String,
}

/// v1.3-Tools-C10 — positional CSV column ordering pin for File.
///
/// MUST mirror the column order in `crates/ga-index/src/schema.rs`
/// `BASE_DDL_STATEMENTS` File CREATE NODE TABLE clause. The CSV emission loop
/// at `build_index` writes cells in this order; lbug's COPY uses `header=false`
/// so positional alignment is the only contract.
///
/// Verified by `crates/ga-query/tests/schema_v4_column_pin.rs::file_columns_const_exists_and_matches_v4_ddl_order`.
pub const FILE_COLUMNS: &[&str] = &[
    "path",
    "lang",
    "size",
    "sha256",
    "modified_at",
    "loc",
    "is_generated",
    "is_vendored",
];

/// v1.3-Tools-C10 — positional CSV column ordering pin for Symbol.
/// MUST mirror Symbol DDL column order in `BASE_DDL_STATEMENTS`.
///
/// v1.3 PR5c1 ships 13 scalars + 2 composites = 15 v4 cols (doc_summary
/// stays deferred). spike_pr5c_store.rs T1-T5 (5/5 PASS) confirmed
/// composite CREATE-with-DEFAULT survives empty-cache reopen + DDL replay
/// — PR2's kuzu#6045 trap was ALTER-ADD-specific, not composite-fundamental.
/// Tools-C13 superseded.
pub const SYMBOL_COLUMNS: &[&str] = &[
    "id",
    "name",
    "file",
    "kind",
    "line",
    "line_end",
    "qualified_name",
    "return_type",
    "arity",
    "is_async",
    "is_override",
    "is_abstract",
    "is_static",
    "is_test_marker",
    "is_generated",
    "confidence",
    "doc_summary",
    "modifiers",
    "params",
];

struct FileRow {
    path: String,
    lang: String,
    size: u64,
    // PR6 — S-005 operational metadata.
    sha256: [u8; 32],
    modified_at_ns: i64,
    loc: i64,
    is_generated: bool,
    is_vendored: bool,
}

/// PR6 — encode bytes as lbug BLOB CSV literal: per-byte `\xHH` escape
/// (Postgres-style bytea). Empirically verified by `spike_blob_csv.rs`
/// Variant F (32 raw bytes round-trip) — bare hex (`deadbeef...`) stores
/// as 64-byte ASCII text not 32-byte Blob.
fn blob_csv_literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        out.push_str(&format!("\\x{b:02x}"));
    }
    out
}

/// PR6 — format Unix epoch seconds + nanoseconds as lbug TIMESTAMP CSV
/// literal. Variant F per `spike_timestamp_csv.rs`: `YYYY-MM-DD hh:mm:ss.zzzzzz`
/// (microseconds, 6 digits) — the engine's own error message lists
/// `[.zzzzzz]` as the optional fractional component. Pure function (no
/// chrono dep). Microsecond precision retained from the captured `mtime_ns`
/// (lossy second→μs is intentional — TIMESTAMP fractional is `[.zzzzzz]`
/// per engine, not nanosecond).
fn format_iso8601_utc(secs: i64, nanos: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let (y, mo, d) = days_to_ymd(days);
    let micros = nanos / 1_000;
    format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.{micros:06}")
}

/// PR6 — convert days-since-epoch to (year, month, day). Civil-from-days
/// algorithm (Howard Hinnant) — handles negative days too. Pure.
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// PR6 — heuristic: path patterns commonly indicating generated code.
/// Universal-truth — based on conventions used by major code generators.
fn is_generated_path(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    if lower.contains("/generated/") || lower.starts_with("generated/") {
        return true;
    }
    // Common per-tool suffixes.
    if lower.ends_with(".pb.go")          // protoc-go
        || lower.ends_with("_pb2.py")     // protoc-py
        || lower.ends_with(".pb.cc")
        || lower.ends_with(".pb.h")
        || lower.ends_with(".gen.go")
        || lower.ends_with(".gen.ts")
        || lower.ends_with(".g.dart")
        || lower.ends_with(".freezed.dart")
        || lower.ends_with("_grpc.pb.go")
        || lower.ends_with(".g.cs")
        || lower.contains(".generated.")
    {
        return true;
    }
    false
}

/// PR6 — heuristic: path patterns commonly indicating vendored / 3rd-party code.
fn is_vendored_path(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    let prefixes = [
        "node_modules/",
        "vendor/",
        "third_party/",
        "third-party/",
        ".cargo/",
        "bower_components/",
    ];
    if prefixes.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    // Embedded paths (e.g. submodule under a subdir)
    let segments = [
        "/node_modules/",
        "/vendor/",
        "/third_party/",
        "/third-party/",
    ];
    segments.iter().any(|p| lower.contains(p))
}

struct SymbolRow {
    id: String,
    name: String,
    file: String,
    kind: String,
    line: i64,
    line_end: i64,
    // v1.3 PR4 — qualified_name (rebuild-stable identity, S-002 / Tools-C1).
    // Format: `{rel_path}::{lang_specific_chain}` where chain comes from
    // `LanguageSpec::format_qualified_name`. Indexer dedup appends `#dup<N>`
    // on collision per AS-006 / Tools-C5.
    qualified_name: String,
    // v1.3 PR5a — function/method parameter count (S-003 / Tools-C2).
    // -1 = unknown sentinel (non-function symbols). 0 = nullary. Populated
    // from `LanguageSpec::extract_arity` per emitted symbol.
    arity: i64,
    // v1.3 PR5b — function return type as raw source text (S-003 / Tools-C2).
    // '' = empty sentinel for non-functions, dynamic langs, unannotated.
    // Populated from `LanguageSpec::extract_return_type`.
    return_type: String,
    // v1.3 PR5c1 — denormalized modifiers (Rust pub/async, Java public/static,
    // etc.). Empty list in PR5c1 (per-lang extractors deferred to PR5c2).
    // Stored as STRING[] composite via lbug CSV `"[a,b]"` syntax.
    modifiers: Vec<String>,
    // v1.3 PR5c1 — function parameters as STRUCT(name,type,default_value)[].
    // Empty in PR5c1; per-lang `extract_params` deferred to PR5c2.
    params: Vec<ga_parser::ParsedParam>,
    // v1.3 PR3 — denormalized SymbolAttribute bools + confidence. v4 cols
    // return_type / arity / doc_summary stay at DDL DEFAULT (PR5 scope).
    // is_test_marker stays false (PR-deferred — needs new per-lang
    // test-attribute extractor).
    is_async: bool,
    is_override: bool,
    is_abstract: bool,
    is_static: bool,
    is_test_marker: bool,
    is_generated: bool,
    confidence: f64,
    /// v1.4 (S-001a / H1 fix) — true when `is_override=true` AND the
    /// indexer's parent-method resolution failed to find an in-repo
    /// target (vendored / external base class). Tools-C12 preserved:
    /// no synthetic OVERRIDES edge written; flag is the rescue signal
    /// for `ga_dead_code` external-parent FP class. AT-014 invariant:
    /// has_unresolved_override=true MUST imply is_override=true.
    has_unresolved_override: bool,
}

/// Cheap guard against call-extractor outputs that aren't legal identifiers.
/// Mirrors the Tools-C9-d allowlist so what the indexer stores stays
/// query-able under the same safety rules.
fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 512
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.' | ':'))
}

fn symbol_kind_str(k: ga_core::SymbolKind) -> String {
    match k {
        ga_core::SymbolKind::Function => "function",
        ga_core::SymbolKind::Method => "method",
        ga_core::SymbolKind::Class => "class",
        ga_core::SymbolKind::Interface => "interface",
        ga_core::SymbolKind::Struct => "struct",
        ga_core::SymbolKind::Enum => "enum",
        ga_core::SymbolKind::Trait => "trait",
        ga_core::SymbolKind::Module => "module",
        ga_core::SymbolKind::Other => "other",
    }
    .to_string()
}
