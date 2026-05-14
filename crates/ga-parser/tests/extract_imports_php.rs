//! v1.2-php S-001 AS-002 — PHP IMPORTS extraction.
//!
//! Five import shapes (corrected node-kind per AS-016):
//! 1. namespace_definition           — `namespace App\Service;`
//! 2. namespace_use_declaration      — `use App\Service\UserRepository;`
//! 3. namespace_use_group inside (2) — `use App\Util\{Logger, Cache};`
//! 4. function/const modifier in (2) — `use function strlen;`
//! 5. use_as_clause inside (2)       — `use X as Y;`

use ga_core::Lang;
use ga_parser::extract_imports;

fn imports_of(src: &[u8]) -> Vec<ga_parser::ParsedImport> {
    extract_imports(Lang::Php, src).expect("extract_imports Ok")
}

#[test]
fn namespace_definition_emits_target_path_no_imported_names() {
    let src = b"<?php namespace App\\Service;";
    let imports = imports_of(src);
    let ns = imports
        .iter()
        .find(|i| i.target_path.contains("App"))
        .unwrap_or_else(|| panic!("namespace not emitted: {imports:?}"));
    assert!(
        ns.target_path == "App\\Service" || ns.target_path == "App\\\\Service",
        "target_path should be the namespace path, got '{}'",
        ns.target_path
    );
    assert!(
        ns.imported_names.is_empty(),
        "namespace_definition binds no specific names: {:?}",
        ns.imported_names
    );
}

#[test]
fn explicit_use_emits_fqn_and_trailing_name() {
    let src = b"<?php\nuse App\\Service\\UserRepository;";
    let imports = imports_of(src);
    let imp = imports
        .iter()
        .find(|i| i.imported_names.iter().any(|n| n == "UserRepository"))
        .unwrap_or_else(|| panic!("UserRepository import not emitted: {imports:?}"));
    assert!(
        imp.target_path.ends_with("UserRepository"),
        "target_path must be the full FQN, got '{}'",
        imp.target_path
    );
}

#[test]
fn group_use_emits_each_name_in_group() {
    let src = b"<?php\nuse App\\Util\\{Logger, Cache};";
    let imports = imports_of(src);
    let names: Vec<String> = imports
        .iter()
        .flat_map(|i| i.imported_names.iter().cloned())
        .collect();
    assert!(
        names.contains(&"Logger".to_string()),
        "group import must bind Logger: {names:?}"
    );
    assert!(
        names.contains(&"Cache".to_string()),
        "group import must bind Cache: {names:?}"
    );
}

#[test]
fn use_function_binds_function_name() {
    let src = b"<?php\nuse function strlen;";
    let imports = imports_of(src);
    let names: Vec<String> = imports
        .iter()
        .flat_map(|i| i.imported_names.iter().cloned())
        .collect();
    assert!(
        names.contains(&"strlen".to_string()),
        "use function strlen must bind strlen: {names:?}"
    );
}

#[test]
fn use_const_binds_const_name() {
    let src = b"<?php\nuse const PHP_INT_MAX;";
    let imports = imports_of(src);
    let names: Vec<String> = imports
        .iter()
        .flat_map(|i| i.imported_names.iter().cloned())
        .collect();
    assert!(
        names.contains(&"PHP_INT_MAX".to_string()),
        "use const PHP_INT_MAX must bind PHP_INT_MAX: {names:?}"
    );
}

#[test]
fn aliased_use_binds_local_alias_not_original() {
    let src = b"<?php\nuse App\\Auth\\AuthService as AuthSvc;";
    let imports = imports_of(src);
    let names: Vec<String> = imports
        .iter()
        .flat_map(|i| i.imported_names.iter().cloned())
        .collect();
    // Local binding is the alias.
    assert!(
        names.contains(&"AuthSvc".to_string()),
        "aliased use must bind alias AuthSvc: {names:?}"
    );
}
