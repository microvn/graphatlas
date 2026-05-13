//! v1.1-M4 S-002b — Lang-C2 fixture-first contract for Kotlin.
//!
//! Pins existence + parseability of the
//! `tests/fixtures/kotlin-tiny/android-mini/` corpus that v1.1-Lang-C2
//! mandates. Integration tests that exercise AS-007 (extension fns) /
//! AS-008 (suspend) / AS-002-equiv (EXTENDS) / AS-003-equiv (IMPORTS) /
//! AS-004-equiv (@Inject REFERENCES) in S-002c can rely on the fixture
//! being on disk and well-formed.

use ga_core::Lang;
use ga_parser::extract_calls;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("kotlin-tiny")
        .join("android-mini")
}

#[test]
fn kotlin_tiny_android_mini_files_exist() {
    let root = fixture_root();
    for f in &[
        "User.kt",
        "UserRepository.kt",
        "UserService.kt",
        "build.gradle.kts",
    ] {
        let p = root.join(f);
        assert!(
            p.exists(),
            "Lang-C2 fixture missing: {} (android-mini fixture is required by graphatlas-v1.1-languages.md S-002 §Data)",
            p.display()
        );
    }
}

#[test]
fn kotlin_tiny_files_parse_without_error() {
    let root = fixture_root();
    for f in &["User.kt", "UserRepository.kt", "UserService.kt"] {
        let bytes = std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("read {f}: {e}"));
        let calls = extract_calls(Lang::Kotlin, &bytes)
            .unwrap_or_else(|e| panic!("extract_calls on {f}: {e:?}"));
        // Sanity: UserService.kt has at least 2 call_expressions
        // (`userRepository.findById(...)`, `userRepository.save(u)`,
        // `User(...)`, `startsWith(...)`). User.kt may have 1 (User(name)
        // primary-constructor params don't count). UserRepository.kt has
        // `mutableMapOf()` + `User(...)` + `delay(10)`.
        if *f == "UserService.kt" {
            assert!(
                !calls.is_empty(),
                "UserService.kt should yield ≥1 call: {calls:?}"
            );
        }
    }
}
