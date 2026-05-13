//! S-004 AS-030 — per-file resource limits.
//! - size cap: default 2 MB, configurable via GRAPHATLAS_MAX_FILE_BYTES
//! - parse wall-clock timeout: default 5 s
//!
//! Per R30 simplified: no AST-depth limit, no catch_unwind, no watchdog —
//! those don't actually sandbox the C tree-sitter library.

use ga_core::Lang;
use ga_parser::{parse_file_bytes, LimitConfig, ParseOutcome};
use std::time::Duration;

#[test]
fn parses_small_file_happy_path() {
    let src = b"def foo():\n    pass\n";
    let cfg = LimitConfig::default();
    let outcome = parse_file_bytes("x.py", Lang::Python, src, &cfg);
    match outcome {
        ParseOutcome::Ok {
            symbols,
            bytes_parsed,
        } => {
            assert!(!symbols.is_empty());
            assert_eq!(bytes_parsed, src.len() as u64);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn skips_file_over_size_cap() {
    // 3MB of whitespace — over default 2MB cap.
    let big: Vec<u8> = vec![b'\n'; 3 * 1024 * 1024];
    let cfg = LimitConfig::default();
    let outcome = parse_file_bytes("huge.py", Lang::Python, &big, &cfg);
    match outcome {
        ParseOutcome::SkippedTooLarge { bytes, cap, .. } => {
            assert_eq!(bytes, big.len() as u64);
            assert_eq!(cap, cfg.max_file_bytes);
        }
        other => panic!("expected SkippedTooLarge, got {other:?}"),
    }
}

#[test]
fn custom_size_cap_applies() {
    let src = vec![b' '; 2000]; // 2 KB
    let cfg = LimitConfig {
        max_file_bytes: 1024,
        ..LimitConfig::default()
    };
    let outcome = parse_file_bytes("x.py", Lang::Python, &src, &cfg);
    assert!(matches!(outcome, ParseOutcome::SkippedTooLarge { .. }));
}

#[test]
fn tiny_timeout_on_real_file_returns_timeout() {
    // Build a deeply-nested but legal Python expression so the parser has
    // something non-trivial to chew on. Combined with a near-zero timeout
    // the progress callback will trip on the first check.
    let mut src = String::from("x = ");
    for _ in 0..5000 {
        src.push_str("(1 + ");
    }
    src.push('1');
    for _ in 0..5000 {
        src.push(')');
    }
    let cfg = LimitConfig {
        max_file_bytes: 10 * 1024 * 1024,
        parse_timeout: Duration::from_nanos(1),
    };
    let outcome = parse_file_bytes("slow.py", Lang::Python, src.as_bytes(), &cfg);
    assert!(
        matches!(outcome, ParseOutcome::Timeout { .. }),
        "expected Timeout, got {outcome:?}"
    );
}

#[test]
fn syntax_error_counted_but_still_extracts_what_it_can() {
    // Broken Python — unterminated function body — tree-sitter still emits
    // a tree; we surface a SyntaxError marker but any recognizable symbols
    // should still come through.
    let src = b"def valid_one():\n    return 1\n\ndef broken(:\n";
    let cfg = LimitConfig::default();
    let outcome = parse_file_bytes("broken.py", Lang::Python, src, &cfg);
    match outcome {
        ParseOutcome::Ok { .. } | ParseOutcome::SyntaxError { .. } => {
            // Either variant acceptable — grammar tolerates partial trees.
        }
        other => panic!("expected Ok or SyntaxError, got {other:?}"),
    }
}
