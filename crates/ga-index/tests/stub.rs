//! S-001 stub contract — kept green across S-003 because Store now WORKS,
//! but schema_version constant guarantee is stable v1 contract.

use ga_index::Store;

#[test]
fn schema_version_is_defined_positive() {
    // Schema starts at 1 and bumps with each breaking change. Foundation-C15
    // bumped to 2 when REFERENCES rel table landed. Test guards that the
    // constant stays present + > 0 rather than a frozen value.
    const _: () = assert!(ga_index::SCHEMA_VERSION >= 1);
    let _ = Store::open; // type-level: public API didn't regress.
}
