//! Tools S-003 cluster A — extract import sites from source for all 5 langs.

use ga_core::Lang;
use ga_parser::extract_imports;

#[test]
fn python_from_import_captures_module_and_line() {
    let src = "from utils.format import fmt\nfrom os import path\n";
    let imports = extract_imports(Lang::Python, src.as_bytes()).unwrap();
    let paths: Vec<&str> = imports.iter().map(|i| i.target_path.as_str()).collect();
    assert!(paths.contains(&"utils.format"), "{paths:?}");
    assert!(paths.contains(&"os"), "{paths:?}");
    // First line in source.
    let utils = imports
        .iter()
        .find(|i| i.target_path == "utils.format")
        .unwrap();
    assert_eq!(utils.import_line, 1);
}

#[test]
fn python_plain_import_captures_module() {
    let src = "import utils.format\n";
    let imports = extract_imports(Lang::Python, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].target_path, "utils.format");
}

#[test]
fn typescript_import_from_path_is_captured() {
    let src = "import { fmt } from './utils/format';\n";
    let imports = extract_imports(Lang::TypeScript, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].target_path, "./utils/format");
}

#[test]
fn empty_source_yields_no_imports() {
    let imports = extract_imports(Lang::Python, b"").unwrap();
    assert!(imports.is_empty());
}

// ---- Cluster B: imported_names + re-export flag -------------------------

#[test]
fn python_from_import_captures_names() {
    let src = "from utils.format import fmt, other\n";
    let imports = extract_imports(Lang::Python, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    let mut names = imports[0].imported_names.clone();
    names.sort();
    assert_eq!(names, vec!["fmt".to_string(), "other".to_string()]);
    assert!(!imports[0].is_re_export);
}

#[test]
fn ts_named_import_captures_names() {
    let src = "import { a, b } from './x';\n";
    let imports = extract_imports(Lang::TypeScript, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    let mut names = imports[0].imported_names.clone();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    assert!(!imports[0].is_re_export);
}

#[test]
fn ts_star_reexport_flagged() {
    let src = "export * from './bar';\n";
    let imports = extract_imports(Lang::TypeScript, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    assert!(imports[0].is_re_export);
    assert_eq!(imports[0].target_path, "./bar");
}

#[test]
fn ts_named_reexport_flagged() {
    let src = "export { X } from './bar';\n";
    let imports = extract_imports(Lang::TypeScript, src.as_bytes()).unwrap();
    assert_eq!(imports.len(), 1);
    assert!(imports[0].is_re_export);
    assert_eq!(imports[0].target_path, "./bar");
    assert!(
        imports[0].imported_names.contains(&"X".to_string()),
        "{:?}",
        imports[0].imported_names
    );
}

#[test]
fn ts_plain_export_is_not_reexport() {
    // `export function foo() {}` has no source string — must NOT emit an
    // import entry.
    let src = "export function foo() { return 1; }\n";
    let imports = extract_imports(Lang::TypeScript, src.as_bytes()).unwrap();
    assert!(
        imports.is_empty(),
        "non-reexport export should not produce import entry: {imports:?}"
    );
}
