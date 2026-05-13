//! v1.1-M4 S-004b Lang-C2 — Ruby tiny fixture sanity.
//!
//! Pin the per-fixture contract: every `.rb` file under `ruby-tiny/` parses
//! Ok and surfaces ≥1 ParsedCall. Bench dataset is separate (S-004-bench);
//! this fixture is the dynamic half of the AS-016 grammar pin (static half
//! = `cargo_pin_strict.rs::cargo_toml_includes_ruby_grammar`).

use ga_core::Lang;
use ga_parser::{extract_calls, parse_source};
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ruby-tiny/sinatra-mini")
}

#[test]
fn ruby_tiny_fixture_files_exist() {
    let dir = fixture_dir();
    for required in &[
        "user.rb",
        "user_repository.rb",
        "user_service.rb",
        "Gemfile",
    ] {
        let path = dir.join(required);
        assert!(
            path.exists(),
            "Lang-C2 ruby-tiny fixture missing required file: {}",
            path.display()
        );
    }
}

#[test]
fn ruby_tiny_files_parse_and_emit_records() {
    let dir = fixture_dir();
    for rb in &["user.rb", "user_repository.rb", "user_service.rb"] {
        let src = fs::read(dir.join(rb)).unwrap();

        let syms = parse_source(Lang::Ruby, &src)
            .unwrap_or_else(|e| panic!("parse_source failed on {rb}: {e:?}"));
        assert!(
            !syms.is_empty(),
            "Lang-C2: {rb} produced zero ParsedSymbol — fixture or extractor broken"
        );

        let calls =
            extract_calls(Lang::Ruby, &src).unwrap_or_else(|e| panic!("extract_calls {rb}: {e:?}"));
        assert!(
            !calls.is_empty(),
            "Lang-C2: {rb} produced zero ParsedCall — fixture or extractor broken"
        );
    }
}
