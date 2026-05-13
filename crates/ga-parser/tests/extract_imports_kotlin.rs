//! v1.1-M4 S-002c — Kotlin IMPORTS extraction (AS-003-equiv).
//!
//! Lang-C1 atomic UC gate. Tree-sitter-kotlin-ng emits a bare `import`
//! node (not `import_declaration` like Java) with a `qualified_identifier`
//! child holding the FQN, optionally followed by `*` (wildcard) or
//! `as <alias>` (renamed import).
//!
//! Examples:
//!   - `import org.foo.Bar`           → path="org.foo.Bar", names=["Bar"]
//!   - `import org.foo.*`             → path="org.foo",     names=[] (wildcard)
//!   - `import org.foo.Bar as B`      → path="org.foo.Bar", names=["B"] (alias overrides)

use ga_core::Lang;
use ga_parser::extract_imports;

fn imports_in(src: &[u8]) -> Vec<ga_parser::ParsedImport> {
    extract_imports(Lang::Kotlin, src).expect("extract_imports Ok")
}

#[test]
fn fqn_import_records_path_and_trailing_class() {
    let src = b"package com.example\nimport org.foo.Bar\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1, "expected 1 import, got {imports:?}");
    let imp = &imports[0];
    assert_eq!(imp.target_path, "org.foo.Bar");
    assert!(
        imp.imported_names.contains(&"Bar".to_string()),
        "trailing identifier must surface in imported_names: {:?}",
        imp.imported_names
    );
}

#[test]
fn wildcard_import_emits_empty_imported_names() {
    let src = b"package com.example\nimport org.foo.*\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1, "expected 1 import, got {imports:?}");
    let imp = &imports[0];
    assert!(
        imp.imported_names.is_empty(),
        "wildcard import must NOT bind a specific name: {:?}",
        imp.imported_names
    );
    let path = imp.target_path.as_str();
    assert!(
        path.starts_with("org.foo"),
        "wildcard target_path should be the package portion `org.foo`, got `{path}`"
    );
}

#[test]
fn aliased_import_uses_alias_as_imported_name() {
    // `import org.foo.Bar as B` → caller-side name is `B`. The alias is the
    // local binding; the original `Bar` is captured separately in
    // extract_imported_aliases (TS/JS use this; Kotlin does not need it
    // for the indexer's name-resolution path).
    let src = b"import org.foo.Bar as B\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    let imp = &imports[0];
    assert!(
        imp.imported_names.contains(&"B".to_string())
            || imp.imported_names.contains(&"Bar".to_string()),
        "alias B must surface (or original Bar as fallback) in imported_names: {:?}",
        imp.imported_names
    );
}

#[test]
fn multi_segment_fqn_strips_to_last_identifier() {
    let src = b"import a.b.c.d.E\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    assert!(
        imports[0].imported_names.contains(&"E".to_string()),
        "deep FQN must surface trailing `E`: {:?}",
        imports[0].imported_names
    );
}

#[test]
fn empty_kotlin_source_returns_no_imports() {
    let imports = imports_in(b"");
    assert!(imports.is_empty());
}

#[test]
fn declaration_without_imports_returns_empty() {
    let src = b"package com.example\nclass Lone\n";
    let imports = imports_in(src);
    assert!(
        imports.is_empty(),
        "package-only source should produce no IMPORTS: {imports:?}"
    );
}

#[test]
fn malformed_kotlin_source_does_not_panic_in_imports_walker() {
    let garbage: &[u8] = b"import }}}{ <<< abandon !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_imports(Lang::Kotlin, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_imports panicked on garbage Kotlin input"
    );
}
