//! v1.1-M4 S-001c — Java IMPORTS extraction (AS-003).
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-003):
//!   Given: `import com.example.auth.User; import com.example.util.*;`
//!   When: indexed
//!   Then: IMPORTS edges emit qualified name for explicit import +
//!         package-level edge for wildcard. `imported_names` populated
//!         per Foundation-C16.

use ga_core::Lang;
use ga_parser::extract_imports;

#[test]
fn explicit_import_target_path_is_full_fqn() {
    let src = b"\
package com.example;\n\
import com.example.auth.User;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    assert!(
        imports
            .iter()
            .any(|i| i.target_path == "com.example.auth.User"),
        "explicit import must surface FQN as target_path: {imports:?}"
    );
}

#[test]
fn explicit_import_imported_names_includes_trailing_class() {
    let src = b"\
package com.example;\n\
import com.example.auth.User;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    let user = imports
        .iter()
        .find(|i| i.target_path == "com.example.auth.User")
        .expect("import for User must exist");
    assert!(
        user.imported_names.contains(&"User".to_string()),
        "imported_names must include trailing class `User` (Foundation-C16): {:?}",
        user.imported_names
    );
}

#[test]
fn wildcard_import_target_path_keeps_package() {
    // `import com.example.util.*;` → target_path = "com.example.util"
    // (the package). Caller decides how to surface the wildcard form.
    let src = b"\
package com.example;\n\
import com.example.util.*;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    assert!(
        imports.iter().any(|i| i.target_path == "com.example.util"),
        "wildcard import must surface package as target_path: {imports:?}"
    );
}

#[test]
fn wildcard_import_emits_no_imported_name() {
    // Wildcard imports don't bind a specific name — `imported_names` is
    // empty for these.
    let src = b"\
package com.example;\n\
import com.example.util.*;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    let pkg = imports
        .iter()
        .find(|i| i.target_path == "com.example.util")
        .expect("wildcard import must exist");
    assert!(
        pkg.imported_names.is_empty(),
        "wildcard imports must NOT populate imported_names: {:?}",
        pkg.imported_names
    );
}

#[test]
fn static_import_picks_up_member_name() {
    // `import static java.util.Collections.emptyList;` → imported_names
    // = ["emptyList"], target_path captures the FQN.
    let src = b"\
package com.example;\n\
import static java.util.Collections.emptyList;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    let stat = imports
        .iter()
        .find(|i| i.target_path.contains("emptyList") || i.target_path.contains("Collections"))
        .unwrap_or_else(|| panic!("no static import surfaced: {imports:?}"));
    assert!(
        stat.imported_names.contains(&"emptyList".to_string())
            || stat.imported_names.contains(&"Collections".to_string()),
        "static import must populate imported_names with the bound member: {:?}",
        stat.imported_names,
    );
}

#[test]
fn multiple_imports_each_emit_separate_record() {
    let src = b"\
package com.example;\n\
import com.example.auth.User;\n\
import com.example.repo.UserRepository;\n\
import java.util.Optional;\n\
public class App {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    let paths: Vec<&str> = imports.iter().map(|i| i.target_path.as_str()).collect();
    for required in &[
        "com.example.auth.User",
        "com.example.repo.UserRepository",
        "java.util.Optional",
    ] {
        assert!(
            paths.contains(required),
            "missing import {required}: {paths:?}"
        );
    }
}

#[test]
fn no_imports_in_source_returns_empty() {
    let src = b"\
public class Standalone {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    assert!(imports.is_empty());
}

// ─── Edge-Case Compliance (skill Phase 1 — added during S-001c cleanup) ───
//
// AS-003 happy-path tests above cover Empty + (degenerate) Boundary.
// Three tests below close Error / Boundary-deep / Special-chars rows.

#[test]
fn malformed_import_does_not_panic() {
    // Error path (R12 contract): broken `import ;` must not panic the
    // walker. tree-sitter-java emits an ERROR node; the extractor either
    // returns Ok with the valid imports or Ok with empty — never panics.
    let garbage: &[u8] = b"package x; import ; import .;; \x00\x01 public class A {}";
    let result = std::panic::catch_unwind(|| extract_imports(Lang::Java, garbage));
    assert!(
        result.is_ok(),
        "extract_imports panicked on malformed import"
    );
}

#[test]
fn deeply_nested_fqn_returns_trailing_class_in_imported_names() {
    // Boundary: a deeply-nested FQN (8 segments) still resolves to
    // a single trailing class in `imported_names`. Pins that the
    // splitter doesn't mis-handle long dotted paths.
    let src = b"\
package x;\n\
import a.b.c.d.e.f.g.h.Deep;\n\
public class A {}\n";
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    let deep = imports
        .iter()
        .find(|i| i.target_path == "a.b.c.d.e.f.g.h.Deep")
        .unwrap_or_else(|| panic!("deep FQN target_path missing: {imports:?}"));
    assert_eq!(
        deep.imported_names,
        vec!["Deep".to_string()],
        "deeply nested FQN must surface only the trailing class: {:?}",
        deep.imported_names
    );
}

#[test]
fn unicode_in_source_does_not_corrupt_import_extraction() {
    // Special chars: a Unicode comment alongside ASCII imports must not
    // disturb the import walk. (Pins the conservative contract; the
    // tree-sitter-java grammar's exact Unicode-identifier semantics
    // are not asserted here — see extract_extends_java.rs sibling test.)
    let src = "// 日本語コメント\npackage x;\nimport com.example.User;\n".as_bytes();
    let imports = extract_imports(Lang::Java, src).expect("Ok");
    assert!(
        imports.iter().any(|i| i.target_path == "com.example.User"),
        "ASCII import must extract cleanly with Unicode comment in file: {imports:?}"
    );
}
