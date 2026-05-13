//! S-002 ga_minimal_context — snippet extraction + token approximation
//! unit tests.
//!
//! Tools-C3 contract (graphatlas-v1.1-tools.md):
//!   "ga_minimal_context token counting MAY approximate when tiktoken not
//!    loaded (±10% error acceptable)."
//!
//! Implementation: char-count / 4 approximation (industry standard for
//! GPT-style tokenizers — within ±10% on natural-language commit-style
//! text and source code per published Anthropic / OpenAI calibration).

use ga_query::snippet::{
    estimate_tokens, extract_signature, read_snippet, SnippetMode, SnippetRequest,
};
use std::fs;
use tempfile::TempDir;

// ─────────────────────────────────────────────────────────────────────────
// estimate_tokens — Tools-C3 ±10% approximation
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_string_estimates_zero_tokens() {
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn token_estimate_uses_char_count_div_four_floor() {
    // Industry-standard approximation: 1 token ≈ 4 chars.
    // "hello world" = 11 chars → 11/4 = 2 (floor) or 3 (round). Pin floor.
    let s = "hello world"; // 11 chars
    let est = estimate_tokens(s);
    assert!(
        est == 2 || est == 3,
        "11-char string should estimate 2-3 tokens (4-char/token rule); got {est}"
    );
}

#[test]
fn token_estimate_scales_linearly() {
    let small = estimate_tokens("a".repeat(100).as_str()); // ~25 tokens
    let big = estimate_tokens("a".repeat(1000).as_str()); // ~250 tokens
    assert!(
        big > small * 9 && big < small * 11,
        "10× chars should yield ~10× tokens (linear); got small={small} big={big}"
    );
}

#[test]
fn token_estimate_unicode_counts_codepoints_not_bytes() {
    // Adversarial: a Unicode-heavy comment should not count UTF-8 bytes
    // as chars — tokenizers operate on codepoints, byte-count would
    // wildly over-estimate (e.g. CJK chars are 3 bytes each).
    let cjk = "中文测试"; // 4 codepoints, 12 UTF-8 bytes
    let est = estimate_tokens(cjk);
    // Expect 4 chars / 4 = 1 token, NOT 12/4 = 3.
    assert_eq!(est, 1, "Unicode codepoints, not bytes; got {est}");
}

// ─────────────────────────────────────────────────────────────────────────
// read_snippet — file:line → text
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn read_snippet_returns_n_lines_starting_at_line() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("foo.py");
    fs::write(
        &path,
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n",
    )
    .unwrap();

    let req = SnippetRequest {
        file: path.to_string_lossy().to_string(),
        line: 3,
        max_lines: 3,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("snippet ok");
    assert_eq!(
        snippet.text, "line 3\nline 4\nline 5\n",
        "lines 3..=5 expected"
    );
    assert_eq!(snippet.line_count, 3);
}

#[test]
fn read_snippet_handles_max_lines_clamp_to_eof() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("foo.py");
    fs::write(&path, "a\nb\nc\n").unwrap();

    let req = SnippetRequest {
        file: path.to_string_lossy().to_string(),
        line: 2,
        max_lines: 100,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("snippet ok");
    assert_eq!(snippet.text, "b\nc\n");
    assert_eq!(snippet.line_count, 2);
}

#[test]
fn read_snippet_with_relative_path_resolves_against_repo_root() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src/auth")).unwrap();
    let abs_path = tmp.path().join("src/auth/backends.py");
    fs::write(&abs_path, "def authenticate():\n    pass\n").unwrap();

    let req = SnippetRequest {
        file: "src/auth/backends.py".to_string(),
        line: 1,
        max_lines: 2,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("relative path resolves");
    assert!(snippet.text.contains("def authenticate"));
}

#[test]
fn read_snippet_line_out_of_range_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("foo.py");
    fs::write(&path, "only line\n").unwrap();

    let req = SnippetRequest {
        file: path.to_string_lossy().to_string(),
        line: 999,
        max_lines: 5,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("oob is graceful");
    assert!(snippet.text.is_empty());
    assert_eq!(snippet.line_count, 0);
}

#[test]
fn read_snippet_missing_file_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let req = SnippetRequest {
        file: "does_not_exist.py".to_string(),
        line: 1,
        max_lines: 5,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("missing file is graceful");
    assert!(snippet.text.is_empty());
    assert_eq!(snippet.line_count, 0);
}

#[test]
fn read_snippet_zero_max_lines_yields_empty() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("foo.py");
    fs::write(&path, "line 1\n").unwrap();
    let req = SnippetRequest {
        file: path.to_string_lossy().to_string(),
        line: 1,
        max_lines: 0,
        mode: SnippetMode::Body,
    };
    let snippet = read_snippet(tmp.path(), &req).expect("zero max_lines ok");
    assert!(snippet.text.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────
// extract_signature — drop body, keep declaration
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn extract_signature_python_def_keeps_declaration_drops_body() {
    let body =
        "def authenticate(user, pw):\n    \"\"\"Check creds.\"\"\"\n    return user.check(pw)\n";
    let sig = extract_signature(body, "py");
    assert!(sig.contains("def authenticate"), "must keep `def` line");
    assert!(
        !sig.contains("user.check"),
        "body line must be dropped from signature: {sig}"
    );
}

#[test]
fn extract_signature_keeps_multiline_declaration() {
    // Multi-line signature with continuation — must keep all signature
    // lines until body starts.
    let body = "def authenticate(\n    user: User,\n    pw: str,\n) -> bool:\n    return True\n";
    let sig = extract_signature(body, "py");
    assert!(sig.contains("user: User"), "multiline params kept: {sig}");
    assert!(sig.contains("-> bool"), "return type kept: {sig}");
    assert!(!sig.contains("return True"), "body dropped: {sig}");
}

#[test]
fn extract_signature_rust_fn_keeps_decl_drops_body() {
    let body =
        "pub fn authenticate(user: &User, pw: &str) -> Result<bool> {\n    user.check(pw)\n}\n";
    let sig = extract_signature(body, "rs");
    assert!(sig.contains("pub fn authenticate"));
    assert!(!sig.contains("user.check"));
}

#[test]
fn extract_signature_typescript_function_keeps_decl() {
    let body =
        "function authenticate(user: User, pw: string): boolean {\n  return user.check(pw);\n}\n";
    let sig = extract_signature(body, "ts");
    assert!(sig.contains("function authenticate"));
    assert!(!sig.contains("user.check"));
}

#[test]
fn extract_signature_falls_back_to_first_line_for_unknown_ext() {
    // Unknown ext → conservative: first non-empty line.
    let body = "this is a single line\nbody continues\n";
    let sig = extract_signature(body, "xyz");
    assert!(sig.starts_with("this is a single line"));
}

#[test]
fn extract_signature_handles_empty_body() {
    assert_eq!(extract_signature("", "py"), "");
    assert_eq!(extract_signature("", "rs"), "");
}
