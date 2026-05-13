//! Parser resource limits (AS-030).
//!
//! Two guards only, per R30:
//!   - byte cap (default 2 MB, env override `GRAPHATLAS_MAX_FILE_BYTES`)
//!   - wall-clock parse timeout (default 5s)
//!
//! The dropped-in-R30 "sandbox" story — catch_unwind, AST-depth limit, memory
//! watchdog — is intentionally absent because none of those actually isolate
//! tree-sitter's C library from segfaulting the process. Real OS isolation
//! lands in v1.1 if an exploit is demonstrated.

use crate::{parse_source, LanguageSpec, ParsedSymbol};
use ga_core::Lang;
use std::time::{Duration, Instant};
use tree_sitter::{Parser, Tree};

/// Default max per-file size = 2 MB.
pub const MAX_FILE_BYTES_DEFAULT: u64 = 2 * 1024 * 1024;

/// Default parse wall-clock timeout = 5 s.
pub const PARSE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(5);

/// Resource budget for a single parse call.
#[derive(Debug, Clone)]
pub struct LimitConfig {
    pub max_file_bytes: u64,
    pub parse_timeout: Duration,
}

impl LimitConfig {
    /// Resolve defaults, honouring `GRAPHATLAS_MAX_FILE_BYTES` env var if set.
    pub fn from_env() -> Self {
        let max_file_bytes = std::env::var("GRAPHATLAS_MAX_FILE_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(MAX_FILE_BYTES_DEFAULT);
        Self {
            max_file_bytes,
            parse_timeout: PARSE_TIMEOUT_DEFAULT,
        }
    }
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FILE_BYTES_DEFAULT,
            parse_timeout: PARSE_TIMEOUT_DEFAULT,
        }
    }
}

/// Outcome of a bounded parse call. Keeps the file-level accounting explicit
/// so a caller can decide whether to count toward the AS-011 failure rate.
#[derive(Debug)]
pub enum ParseOutcome {
    /// Parse succeeded with no grammar errors.
    Ok {
        symbols: Vec<ParsedSymbol>,
        bytes_parsed: u64,
    },
    /// File exceeded [`LimitConfig::max_file_bytes`]. Not counted as a parse
    /// failure for AS-011 threshold — it's a policy decision.
    SkippedTooLarge { path: String, bytes: u64, cap: u64 },
    /// Parser didn't finish before [`LimitConfig::parse_timeout`]. Counts as
    /// a parse failure.
    Timeout { path: String, elapsed: Duration },
    /// Tree-sitter returned a tree with an error node. Caller gets partial
    /// symbols plus the failure signal. Counts as a parse failure.
    SyntaxError {
        path: String,
        symbols: Vec<ParsedSymbol>,
        error_count: u32,
    },
}

/// Parse `bytes` as `lang`, enforcing [`LimitConfig`].
pub fn parse_file_bytes(path: &str, lang: Lang, bytes: &[u8], cfg: &LimitConfig) -> ParseOutcome {
    let n = bytes.len() as u64;
    if n > cfg.max_file_bytes {
        crate::logs::emit_size_skip(std::path::Path::new(path), n, cfg.max_file_bytes);
        return ParseOutcome::SkippedTooLarge {
            path: path.to_string(),
            bytes: n,
            cap: cfg.max_file_bytes,
        };
    }

    // Fast-path trivial cases without spinning up a Parser.
    if bytes.is_empty() {
        return ParseOutcome::Ok {
            symbols: Vec::new(),
            bytes_parsed: 0,
        };
    }

    // Use tree-sitter's progress_callback to enforce timeout. The callback
    // is called periodically; returning `true` aborts the parse.
    let pool = crate::ParserPool::new();
    let spec = match pool.spec_for(lang) {
        Some(s) => s,
        None => {
            return ParseOutcome::SyntaxError {
                path: path.to_string(),
                symbols: Vec::new(),
                error_count: 1,
            }
        }
    };

    let mut parser = Parser::new();
    if parser.set_language(&spec.tree_sitter_lang()).is_err() {
        return ParseOutcome::SyntaxError {
            path: path.to_string(),
            symbols: Vec::new(),
            error_count: 1,
        };
    }

    let deadline = Instant::now() + cfg.parse_timeout;
    let mut progress = move |_state: &tree_sitter::ParseState| Instant::now() >= deadline;
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);

    // parse_with_options takes a byte-source callback: given an offset, return
    // the slice starting there. For whole-in-memory parsing this is a one-shot
    // slice handoff.
    let mut reader = |offset: usize, _pos: tree_sitter::Point| -> &[u8] {
        if offset >= bytes.len() {
            &[]
        } else {
            &bytes[offset..]
        }
    };

    let tree = match parser.parse_with_options(&mut reader, None, Some(options)) {
        Some(t) => t,
        None => {
            crate::logs::emit_parse_timeout(std::path::Path::new(path), cfg.parse_timeout);
            return ParseOutcome::Timeout {
                path: path.to_string(),
                elapsed: cfg.parse_timeout,
            };
        }
    };

    // Walk + collect symbols regardless of error nodes so we surface partial
    // results alongside a SyntaxError signal.
    let mut symbols = Vec::new();
    crate::walker::walk_tree(tree.root_node(), bytes, spec, None, &mut symbols);

    if tree.root_node().has_error() {
        let mut error_count: u32 = 0;
        count_error_nodes(tree.root_node(), &mut error_count);
        crate::logs::emit_syntax_error(std::path::Path::new(path), error_count);
        return ParseOutcome::SyntaxError {
            path: path.to_string(),
            symbols,
            error_count,
        };
    }

    ParseOutcome::Ok {
        symbols,
        bytes_parsed: n,
    }
}

fn count_error_nodes(node: tree_sitter::Node<'_>, count: &mut u32) {
    if node.is_error() || node.is_missing() {
        *count += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count_error_nodes(child, count);
    }
}

// Keep the convenience `parse_source` wrapper delegating to the bounded path
// with default limits. Existing callers (extract_symbols tests) keep working.
#[doc(hidden)]
pub fn parse_source_default(lang: Lang, source: &[u8]) -> ga_core::Result<Vec<ParsedSymbol>> {
    parse_source(lang, source)
}

/// Like [`ParseOutcome`] but also yields the parsed `tree_sitter::Tree`
/// alongside symbols, so the indexer can dispatch the 4 edge extractors
/// (calls, references, extends, imports) on the same tree instead of
/// re-parsing the file 5×.
pub enum ParseTreeOutcome<'pool> {
    /// Parse succeeded (possibly with grammar errors — `error_count > 0`
    /// means partial). Caller gets tree + symbols + the resolved spec so the
    /// `extract_*_from_tree` calls don't have to look it up again.
    Ok {
        tree: Tree,
        symbols: Vec<ParsedSymbol>,
        spec: &'pool dyn LanguageSpec,
        error_count: u32,
    },
    /// File exceeded [`LimitConfig::max_file_bytes`].
    Skipped,
}

/// Parse-once helper for the indexer. Mirrors [`parse_file_bytes`] for the
/// size-cap + timeout + syntax-error logging contract, but returns the parsed
/// `Tree` so the 4 edge extractors can share it.
pub fn parse_file_tree<'pool>(
    path: &str,
    lang: Lang,
    bytes: &[u8],
    cfg: &LimitConfig,
    pool: &'pool crate::ParserPool,
) -> ParseTreeOutcome<'pool> {
    let n = bytes.len() as u64;
    if n > cfg.max_file_bytes {
        crate::logs::emit_size_skip(std::path::Path::new(path), n, cfg.max_file_bytes);
        return ParseTreeOutcome::Skipped;
    }

    let Some(spec) = pool.spec_for(lang) else {
        return ParseTreeOutcome::Skipped;
    };

    let mut parser = Parser::new();
    if parser.set_language(&spec.tree_sitter_lang()).is_err() {
        return ParseTreeOutcome::Skipped;
    }

    let deadline = Instant::now() + cfg.parse_timeout;
    let mut progress = move |_state: &tree_sitter::ParseState| Instant::now() >= deadline;
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    let mut reader = |offset: usize, _pos: tree_sitter::Point| -> &[u8] {
        if offset >= bytes.len() {
            &[]
        } else {
            &bytes[offset..]
        }
    };
    let tree = match parser.parse_with_options(&mut reader, None, Some(options)) {
        Some(t) => t,
        None => {
            crate::logs::emit_parse_timeout(std::path::Path::new(path), cfg.parse_timeout);
            return ParseTreeOutcome::Skipped;
        }
    };

    let mut symbols = Vec::new();
    crate::walker::walk_tree(tree.root_node(), bytes, spec, None, &mut symbols);

    let mut error_count: u32 = 0;
    if tree.root_node().has_error() {
        count_error_nodes(tree.root_node(), &mut error_count);
        crate::logs::emit_syntax_error(std::path::Path::new(path), error_count);
    }

    ParseTreeOutcome::Ok {
        tree,
        symbols,
        spec,
        error_count,
    }
}
