//! v1.2-php S-001 AS-003 — PHP EXTENDS extraction.
//!
//! Four shapes (per AS-016 corrected node-kind paths):
//! 1. class extends + implements      — class_declaration.base_clause + class_interface_clause
//! 2. interface extends multiple      — interface_declaration.base_clause (comma-list)
//! 3. trait composition inside class  — class_declaration.declaration_list → use_declaration → use_list
//! 4. qualified base                   — `\App\Foundation\Kernel` → stripped to `Kernel`

use ga_core::Lang;
use ga_parser::extract_extends;

fn bases_of(src: &[u8]) -> Vec<String> {
    extract_extends(Lang::Php, src)
        .expect("extract_extends Ok")
        .into_iter()
        .map(|e| e.base_name)
        .collect()
}

#[test]
fn class_extends_and_implements_emits_one_edge_per_parent() {
    let src = b"<?php\nclass Admin extends User implements Printable, Cloneable {}";
    let bases = bases_of(src);
    assert!(
        bases.contains(&"User".to_string()),
        "missing User: {bases:?}"
    );
    assert!(
        bases.contains(&"Printable".to_string()),
        "missing Printable: {bases:?}"
    );
    assert!(
        bases.contains(&"Cloneable".to_string()),
        "missing Cloneable: {bases:?}"
    );
}

#[test]
fn interface_extends_multiple_parents() {
    let src = b"<?php\ninterface Printable extends Serializable, Cloneable {}";
    let bases = bases_of(src);
    assert!(
        bases.contains(&"Serializable".to_string()),
        "missing Serializable: {bases:?}"
    );
    assert!(
        bases.contains(&"Cloneable".to_string()),
        "missing Cloneable: {bases:?}"
    );
}

#[test]
fn trait_use_inside_class_body_emits_extends_edges() {
    // AS-003 form 3: `class S { use TraitA, TraitB; }` — each trait → 1 EXTENDS edge.
    let src = b"\
<?php
class Service {
    use LoggerTrait, CacheTrait;
}
";
    let bases = bases_of(src);
    assert!(
        bases.contains(&"LoggerTrait".to_string()),
        "missing LoggerTrait: {bases:?}"
    );
    assert!(
        bases.contains(&"CacheTrait".to_string()),
        "missing CacheTrait: {bases:?}"
    );
}

#[test]
fn qualified_base_stripped_to_trailing_identifier() {
    // AS-003 form 4: `class App extends \App\Foundation\Kernel` → base "Kernel".
    let src = b"<?php\nclass App extends \\App\\Foundation\\Kernel {}";
    let bases = bases_of(src);
    assert!(
        bases.contains(&"Kernel".to_string()),
        "qualified base must strip to trailing 'Kernel': {bases:?}"
    );
    assert!(
        !bases.iter().any(|b| b.contains('\\')),
        "qualified namespace must be stripped, found backslash in: {bases:?}"
    );
}
