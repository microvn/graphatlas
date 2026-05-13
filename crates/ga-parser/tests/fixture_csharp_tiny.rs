//! v1.1-M4 S-003b — Lang-C2 fixture-first contract for C#.
//!
//! Pins existence + parseability of `tests/fixtures/csharp-tiny/aspnet-mini/`.
//! Integration tests exercising AS-010 (EXTENDS+interfaces) / AS-011
//! (partial classes) / AS-003-equiv IMPORTS / AS-004-equiv [Inject]
//! REFERENCES in S-003c can rely on the fixture being on disk + well-formed.

use ga_core::Lang;
use ga_parser::extract_calls;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("csharp-tiny")
        .join("aspnet-mini")
}

#[test]
fn csharp_tiny_aspnet_mini_files_exist() {
    let root = fixture_root();
    for f in &[
        "User.cs",
        "UserRepository.cs",
        "UserService.cs",
        "aspnet-mini.csproj",
    ] {
        let p = root.join(f);
        assert!(
            p.exists(),
            "Lang-C2 fixture missing: {} (aspnet-mini fixture is required by graphatlas-v1.1-languages.md S-003 §Data)",
            p.display()
        );
    }
}

#[test]
fn csharp_tiny_files_parse_without_error() {
    let root = fixture_root();
    for f in &["User.cs", "UserRepository.cs", "UserService.cs"] {
        let bytes = std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("read {f}: {e}"));
        let calls = extract_calls(Lang::CSharp, &bytes)
            .unwrap_or_else(|e| panic!("extract_calls on {f}: {e:?}"));
        // Sanity: UserService.cs has at least 2 invocations (`_userRepository.FindById(...)`,
        // `_userRepository.Save(u)`, `Console.WriteLine(...)`, `new User(...)`).
        if *f == "UserService.cs" {
            assert!(
                !calls.is_empty(),
                "UserService.cs should yield ≥1 call: {calls:?}"
            );
        }
    }
}
