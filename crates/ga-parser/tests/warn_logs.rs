//! AS-030/011/031 spec-literal log lines. Pure-function helpers so callers
//! (indexer pipeline, doctor) emit consistent text. Actual stderr emission
//! wired via eprintln! in the production code paths.

use ga_parser::logs::{
    warn_line_parse_timeout, warn_line_size_skip, warn_line_symlink_escape, warn_line_syntax_error,
};

#[test]
fn size_skip_line_matches_as030() {
    let line = warn_line_size_skip("huge.ts", 3 * 1024 * 1024, 2 * 1024 * 1024);
    assert!(line.starts_with("WARN: "), "{line}");
    assert!(line.contains("huge.ts"), "{line}");
    assert!(line.contains("skipped"), "{line}");
    assert!(line.contains("3145728") || line.contains("3.0 MB") || line.contains("3MB"));
}

#[test]
fn parse_timeout_line_matches_as030() {
    let line = warn_line_parse_timeout("slow.py", std::time::Duration::from_secs(5));
    assert!(line.starts_with("WARN: "), "{line}");
    assert!(line.contains("slow.py"), "{line}");
    assert!(line.contains("timeout") || line.contains("5"), "{line}");
}

#[test]
fn syntax_error_line_matches_as011() {
    let line = warn_line_syntax_error("broken.ts", 4);
    assert!(line.starts_with("WARN: "), "{line}");
    assert!(line.contains("broken.ts"), "{line}");
    assert!(line.contains("syntax") || line.contains("error"), "{line}");
}

#[test]
fn symlink_escape_line_matches_as031_literal() {
    // AS-031 literal: "skipped symlink escape: <path> -> <target>"
    let line = warn_line_symlink_escape("docs/ref", "/home/user");
    assert!(line.starts_with("WARN: "), "{line}");
    assert!(
        line.contains("skipped symlink escape"),
        "must match AS-031 literal: {line}"
    );
    assert!(line.contains("docs/ref"), "{line}");
    assert!(line.contains("/home/user"), "{line}");
    assert!(line.contains("->"), "{line}");
}
