//! Ruby IMPORTS extraction — `require` / `require_relative` as import sites.
//!
//! Ruby has no static import statement; both forms are `call` nodes. The
//! LanguageSpec surfaces them as imports (target_path = the required path),
//! powering `ga_architecture` inter-gem edges. `require_relative` paths are
//! normalised to a leading `./` so the resolver can branch relative vs
//! load-path.

use ga_core::Lang;
use ga_parser::extract_imports;

fn imports_of(src: &[u8]) -> Vec<ga_parser::ParsedImport> {
    extract_imports(Lang::Ruby, src).expect("extract_imports Ok")
}

#[test]
fn require_emits_loadpath_target_bare() {
    let src = b"require 'active_support/core_ext'\n";
    let imports = imports_of(src);
    assert!(
        imports
            .iter()
            .any(|i| i.target_path == "active_support/core_ext"),
        "require must emit the bare load-path target: {imports:?}"
    );
}

#[test]
fn require_relative_normalised_to_dot_slash() {
    // Bare-name require_relative must gain a leading ./ so the resolver treats
    // it as relative, not load-path.
    let src = b"require_relative 'foo/bar'\n";
    let imports = imports_of(src);
    assert!(
        imports.iter().any(|i| i.target_path == "./foo/bar"),
        "require_relative bare name must normalise to ./foo/bar: {imports:?}"
    );
}

#[test]
fn require_relative_dot_path_preserved() {
    let src = b"require_relative './foo'\n";
    let imports = imports_of(src);
    assert!(
        imports.iter().any(|i| i.target_path == "./foo"),
        "require_relative './foo' preserved: {imports:?}"
    );
}

#[test]
fn ordinary_call_is_not_an_import() {
    // A non-require call must not be surfaced as an import.
    let src = b"puts 'hello'\nFoo.bar(1)\n";
    let imports = imports_of(src);
    assert!(
        imports.is_empty(),
        "ordinary calls must not be imports: {imports:?}"
    );
}

#[test]
fn receiver_form_require_is_ignored() {
    // `Kernel.require "x"` is a receiver call — not the bare top-level form we
    // resolve. Must not emit (avoids ambiguous receiver semantics).
    let src = b"Kernel.require 'x'\n";
    let imports = imports_of(src);
    assert!(
        !imports.iter().any(|i| i.target_path == "x"),
        "receiver-form require must be ignored: {imports:?}"
    );
}
