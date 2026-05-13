//! S-002 ga_minimal_context — snippet extraction + token approximation.
//!
//! Tools-C3 contract (graphatlas-v1.1-tools.md):
//!   "ga_minimal_context token counting MAY approximate when tiktoken not
//!    loaded (±10% error acceptable)."
//!
//! Implementation: char-count / 4 approximation (industry-standard for
//! GPT-style tokenizers — within ±10% on natural-language commit-style
//! text and source code per published Anthropic / OpenAI calibration).
//! `tiktoken-rs` integration is opt-in future work — the [`estimate_tokens`]
//! API stays stable so a future swap is internal.

use ga_core::Result;
use std::path::{Path, PathBuf};

/// Tokens-per-char approximation factor (chars / N → tokens).
const CHARS_PER_TOKEN: usize = 4;

/// Approximate token count for an arbitrary text snippet.
///
/// Counts Unicode codepoints (NOT UTF-8 bytes) — tokenizers operate on
/// codepoints and a byte-count would over-estimate CJK/emoji-heavy text
/// by 3-4×.
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    (chars / CHARS_PER_TOKEN) as u32
}

/// Snippet extraction mode. `Body` = full N lines starting at line. `Signature`
/// = call [`extract_signature`] on the body to drop method body keep decl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetMode {
    Body,
    Signature,
}

#[derive(Debug, Clone)]
pub struct SnippetRequest {
    pub file: String,
    /// 1-based line number (matches `Symbol.line` shape).
    pub line: u32,
    pub max_lines: u32,
    pub mode: SnippetMode,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snippet {
    pub text: String,
    /// Number of source lines included in `text`.
    pub line_count: u32,
}

impl Snippet {
    pub fn token_estimate(&self) -> u32 {
        estimate_tokens(&self.text)
    }
}

/// Read N lines starting at the given 1-based line. Out-of-range, missing
/// file, and zero `max_lines` all return empty Snippet (graceful degrade
/// per Tools-C1 — never error on read miss; the consumer surfaces the
/// empty-context path via lower budget_used).
pub fn read_snippet(repo_root: &Path, req: &SnippetRequest) -> Result<Snippet> {
    if req.max_lines == 0 || req.line == 0 {
        return Ok(Snippet::default());
    }
    let path = resolve_path(repo_root, &req.file);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(Snippet::default());
    };
    let lines: Vec<&str> = content.lines().collect();
    let start_idx = (req.line as usize).saturating_sub(1);
    if start_idx >= lines.len() {
        return Ok(Snippet::default());
    }
    let end_idx = (start_idx + req.max_lines as usize).min(lines.len());
    let mut text = String::new();
    for line in &lines[start_idx..end_idx] {
        text.push_str(line);
        text.push('\n');
    }
    let line_count = (end_idx - start_idx) as u32;
    let final_text = match req.mode {
        SnippetMode::Body => text,
        SnippetMode::Signature => {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            extract_signature(&text, ext)
        }
    };
    Ok(Snippet {
        text: final_text,
        line_count,
    })
}

/// Per-language signature heuristic. Drops the body of a function/class
/// declaration, keeps the declaration line(s).
///
/// Strategy (covers the v1.1 supported languages — Python/TS/JS/Go/Rust/
/// Java/Kotlin/C#/Ruby):
/// - Python: keep lines up to and including the line ending with `:`
///   (followed only by whitespace), drop everything after.
/// - Brace-langs (rs/ts/js/go/java/kt/cs): keep lines up to the line
///   containing the FIRST opening `{`, drop everything after.
/// - Ruby: keep the first non-empty line (Ruby `def` lines are
///   self-contained; body follows on subsequent lines until `end`).
/// - Unknown ext: first non-empty line as conservative fallback.
pub fn extract_signature(body: &str, ext: &str) -> String {
    if body.is_empty() {
        return String::new();
    }

    match ext {
        "py" => extract_python_signature(body),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "go" | "java" | "kt" | "kts"
        | "cs" => extract_brace_signature(body),
        "rb" => first_nonempty_line(body),
        _ => first_nonempty_line(body),
    }
}

fn extract_python_signature(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        out.push_str(line);
        out.push('\n');
        // Signature ends at the line whose content (after rstrip) ends in `:`.
        if line.trim_end().ends_with(':') {
            return out;
        }
    }
    out
}

fn extract_brace_signature(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        out.push_str(line);
        out.push('\n');
        if line.contains('{') {
            return out;
        }
    }
    out
}

fn first_nonempty_line(body: &str) -> String {
    for line in body.lines() {
        if !line.trim().is_empty() {
            let mut s = line.to_string();
            s.push('\n');
            return s;
        }
    }
    String::new()
}

fn resolve_path(repo_root: &Path, file: &str) -> PathBuf {
    let p = Path::new(file);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        repo_root.join(p)
    }
}
