//! Spec-literal WARN log lines for parser / walker events.
//! Exposed as pure functions so callers (indexer, doctor) emit consistent
//! text; `eprintln!` happens at the callsites inside limits.rs / walk.rs.

use std::path::Path;
use std::time::Duration;

/// AS-030 size cap skip. Emitted when a file exceeds `max_file_bytes`.
pub fn warn_line_size_skip(path: &str, bytes: u64, cap: u64) -> String {
    format!("WARN: skipped {path} ({bytes} bytes > {cap} bytes cap) — see AS-030")
}

/// AS-030 parse timeout. Emitted when tree-sitter's progress_callback tripped.
pub fn warn_line_parse_timeout(path: &str, elapsed: Duration) -> String {
    format!(
        "WARN: parse timeout on {path} after {}s — file skipped, see AS-030",
        elapsed.as_secs()
    )
}

/// AS-011 syntax error. Emitted on `SyntaxError` outcome (tree had error nodes).
pub fn warn_line_syntax_error(path: &str, error_count: u32) -> String {
    format!("WARN: {path} has {error_count} syntax error(s) — partial symbols only, see AS-011")
}

/// AS-031 literal: `skipped symlink escape: <path> -> <target>`.
pub fn warn_line_symlink_escape(rel_path: &str, target: &str) -> String {
    format!("WARN: skipped symlink escape: {rel_path} -> {target}")
}

/// Convenience wrappers that both format AND emit to stderr. Used by
/// production code so test code can verify the pure-format helpers directly.
#[inline]
pub fn emit_size_skip(path: &Path, bytes: u64, cap: u64) {
    eprintln!(
        "{}",
        warn_line_size_skip(&path.display().to_string(), bytes, cap)
    );
}

#[inline]
pub fn emit_parse_timeout(path: &Path, elapsed: Duration) {
    eprintln!(
        "{}",
        warn_line_parse_timeout(&path.display().to_string(), elapsed)
    );
}

#[inline]
pub fn emit_syntax_error(path: &Path, error_count: u32) {
    eprintln!(
        "{}",
        warn_line_syntax_error(&path.display().to_string(), error_count)
    );
}

#[inline]
pub fn emit_symlink_escape(rel_path: &Path, target: &Path) {
    eprintln!(
        "{}",
        warn_line_symlink_escape(
            &rel_path.display().to_string(),
            &target.display().to_string()
        )
    );
}
