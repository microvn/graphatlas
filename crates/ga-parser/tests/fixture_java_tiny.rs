//! v1.1-M4 S-001b — Lang-C2 fixture-first contract for Java.
//!
//! Pins existence + parseability of the `tests/fixtures/java-tiny/spring-mini/`
//! corpus that v1.1-Lang-C2 mandates. Integration tests that exercise
//! AS-002 (EXTENDS) / AS-003 (IMPORTS) / AS-004 (@Autowired REFERENCES)
//! in S-001c can rely on the fixture being on disk and well-formed.

use ga_core::Lang;
use ga_parser::extract_calls;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("java-tiny")
        .join("spring-mini")
}

#[test]
fn java_tiny_spring_mini_files_exist() {
    let root = fixture_root();
    for f in &[
        "User.java",
        "UserRepository.java",
        "UserService.java",
        "pom.xml",
    ] {
        let p = root.join(f);
        assert!(
            p.exists(),
            "Lang-C2 fixture missing: {} (Spring-mini fixture is required by graphatlas-v1.1-languages.md AS-001 §Data)",
            p.display()
        );
    }
}

#[test]
fn java_tiny_files_parse_without_error() {
    let root = fixture_root();
    for f in &["User.java", "UserRepository.java", "UserService.java"] {
        let bytes = std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("read {f}: {e}"));
        let calls = extract_calls(Lang::Java, &bytes)
            .unwrap_or_else(|e| panic!("extract_calls on {f}: {e:?}"));
        // Sanity: UserService.java has at least 2 method invocations
        // (`userRepository.findById(...)`, `userRepository.save(u)`,
        // `System.currentTimeMillis()`, etc.). User/UserRepository may have 0.
        if *f == "UserService.java" {
            assert!(
                !calls.is_empty(),
                "UserService.java should yield ≥1 call: {calls:?}"
            );
        }
    }
}
