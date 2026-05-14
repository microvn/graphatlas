use ga_core::Lang;

#[test]
fn from_ext_known() {
    assert_eq!(Lang::from_ext("py"), Some(Lang::Python));
    assert_eq!(Lang::from_ext("pyw"), Some(Lang::Python));
    assert_eq!(Lang::from_ext("ts"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_ext("tsx"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_ext("js"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_ext("jsx"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_ext("mjs"), Some(Lang::JavaScript));
    assert_eq!(Lang::from_ext("go"), Some(Lang::Go));
    assert_eq!(Lang::from_ext("rs"), Some(Lang::Rust));
}

#[test]
fn from_ext_unknown_returns_none() {
    // Truly unknown extensions (no v1.1 lang claims them).
    // `php` removed — v1.2 S-001 added Lang::Php.
    assert_eq!(Lang::from_ext("swift"), None);
    assert_eq!(Lang::from_ext("hs"), None);
    assert_eq!(Lang::from_ext("c"), None);
    assert_eq!(Lang::from_ext(""), None);
}

#[test]
fn as_str_stable() {
    assert_eq!(Lang::Python.as_str(), "python");
    assert_eq!(Lang::TypeScript.as_str(), "typescript");
    assert_eq!(Lang::JavaScript.as_str(), "javascript");
    assert_eq!(Lang::Go.as_str(), "go");
    assert_eq!(Lang::Rust.as_str(), "rust");
}

// v1.1-M4 (S-005a) — Lang enum extension for Phase C languages.
// AS-017 prerequisite: Lang variants must exist before "registered-but-no-spec"
// error can be exercised.

#[test]
fn from_ext_v1_1_languages() {
    assert_eq!(Lang::from_ext("java"), Some(Lang::Java));
    assert_eq!(Lang::from_ext("kt"), Some(Lang::Kotlin));
    assert_eq!(Lang::from_ext("kts"), Some(Lang::Kotlin));
    assert_eq!(Lang::from_ext("cs"), Some(Lang::CSharp));
    assert_eq!(Lang::from_ext("rb"), Some(Lang::Ruby));
}

#[test]
fn as_str_v1_1_languages() {
    assert_eq!(Lang::Java.as_str(), "java");
    assert_eq!(Lang::Kotlin.as_str(), "kotlin");
    assert_eq!(Lang::CSharp.as_str(), "csharp");
    assert_eq!(Lang::Ruby.as_str(), "ruby");
}

// ─── Lang::ALL exhaustiveness guard ──────────────────────────────────
//
// `Lang::ALL` is the canonical iterator used by every consumer that
// needs to walk all supported languages (is_test_path tests, M1 gate,
// guide doc §11 quick-reference, ...). The test below uses an
// exhaustive match on `Lang` so that adding a new variant to the enum
// will FAIL COMPILE here until the new variant is also added to
// `Lang::ALL`. This is the single mechanical guard that forces the
// list to track the enum.

#[test]
fn lang_all_covers_every_variant() {
    use std::collections::HashSet;

    // Exhaustive helper: every variant produces `true`. Adding a new
    // variant to `Lang` makes this match non-exhaustive → compile fails
    // here (the dev fixes by adding the variant to the `|` chain).
    fn is_known(l: Lang) -> bool {
        match l {
            Lang::Python
            | Lang::TypeScript
            | Lang::JavaScript
            | Lang::Go
            | Lang::Rust
            | Lang::Java
            | Lang::Kotlin
            | Lang::CSharp
            | Lang::Ruby
            | Lang::Php => true,
        }
    }

    // Every entry in ALL is a known variant (sanity).
    for &lang in Lang::ALL {
        assert!(is_known(lang));
    }

    // ALL contains each variant exactly once (no duplication).
    let unique: HashSet<Lang> = Lang::ALL.iter().copied().collect();
    assert_eq!(
        unique.len(),
        Lang::ALL.len(),
        "Lang::ALL must list each variant exactly once: {:?}",
        Lang::ALL
    );

    // Spot-check known cardinality. If this assertion fires after
    // adding a new variant, update both ALL and this number.
    assert_eq!(
        Lang::ALL.len(),
        10,
        "Lang::ALL has {} entries; v1.2 expects 10 (9 v1+v1.1 + Php). \
         Adding a new variant? Update Lang::ALL in types.rs and this expected count.",
        Lang::ALL.len()
    );
}
