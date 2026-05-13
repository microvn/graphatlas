//! v1.1-M4 S-003c — C# IMPORTS extraction (AS-003-equiv).
//!
//! Lang-C1 atomic UC gate. C# `using_directive` covers three syntactic
//! forms — all parse to the same node kind:
//!
//!   - `using System;`               — plain (single identifier or qualified_name)
//!   - `using static System.Math;`   — static (members brought into scope)
//!   - `using F = System.Foo;`       — alias (LHS = local name, RHS = path)
//!
//! C# has NO wildcard import (no `using System.*` form).

use ga_core::Lang;
use ga_parser::extract_imports;

fn imports_in(src: &[u8]) -> Vec<ga_parser::ParsedImport> {
    extract_imports(Lang::CSharp, src).expect("extract_imports Ok")
}

#[test]
fn plain_single_identifier_using() {
    let src = b"using System;\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    let imp = &imports[0];
    assert_eq!(imp.target_path, "System");
    assert!(
        imp.imported_names.contains(&"System".to_string()),
        "plain `using System;` must surface `System` in imported_names: {:?}",
        imp.imported_names
    );
}

#[test]
fn qualified_using_records_full_path_and_trailing_segment() {
    let src = b"using System.Collections.Generic;\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    let imp = &imports[0];
    assert_eq!(imp.target_path, "System.Collections.Generic");
    assert!(
        imp.imported_names.contains(&"Generic".to_string()),
        "qualified `using` must surface trailing `Generic`: {:?}",
        imp.imported_names
    );
}

#[test]
fn static_using_treats_last_segment_as_imported_name() {
    let src = b"using static System.Math;\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    let imp = &imports[0];
    assert_eq!(imp.target_path, "System.Math");
    assert!(
        imp.imported_names.contains(&"Math".to_string()),
        "static `using` must surface containing-type `Math` (members of Math): {:?}",
        imp.imported_names
    );
}

#[test]
fn alias_using_uses_alias_as_imported_name() {
    let src = b"using F = System.Foo;\n";
    let imports = imports_in(src);
    assert_eq!(imports.len(), 1);
    let imp = &imports[0];
    assert_eq!(
        imp.target_path, "System.Foo",
        "target_path must be RHS (the aliased path), got {}",
        imp.target_path
    );
    assert!(
        imp.imported_names.contains(&"F".to_string()),
        "alias `F = System.Foo` must surface LOCAL name `F` in imported_names: {:?}",
        imp.imported_names
    );
}

#[test]
fn empty_source_returns_no_imports() {
    let imports = imports_in(b"");
    assert!(imports.is_empty());
}

#[test]
fn declaration_only_csharp_source_returns_no_imports() {
    let src = b"namespace N { class C {} }\n";
    let imports = imports_in(src);
    assert!(
        imports.is_empty(),
        "namespace+class only must produce no IMPORTS: {imports:?}"
    );
}

#[test]
fn malformed_csharp_source_does_not_panic_in_imports_walker() {
    let garbage: &[u8] = b"using }}}{ <<< abandon !!! \x01\xff\xfe";
    let result = std::panic::catch_unwind(|| extract_imports(Lang::CSharp, garbage));
    assert!(
        result.is_ok(),
        "Lang-C1: extract_imports panicked on garbage C# input"
    );
}
