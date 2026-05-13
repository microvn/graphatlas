// Export the Rust binary's symbols so lbug extensions (dlopen'd at runtime
// from ~/.lbdb/extension/<ver>/<arch>/<ext>/lib*.lbug_extension) can resolve
// engine symbols (`lbug::function::GDSFunction::getPhysicalPlan`,
// `lbug::function::TableFunction::emptyTableFunc`,
// `lbug::json_extension::jsonSchema`, etc.) back into the host binary.
//
// Without this, LOAD EXTENSION fails with `dlopen: symbol not found in flat
// namespace '<mangled lbug symbol>'`. See LadybugDB/ladybug#438 — fix
// confirmed by upstream maintainer (adsharma) 2026-04-30.
//
// Mirrors archive/rust-poc/build.rs which shipped this fix in production.
fn main() {
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg=-Wl,-export_dynamic");

    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,--export-dynamic");

    // v1.5 PR3 foundation S-002 AS-006: Windows arm.
    //
    // MSVC linker (link.exe) does NOT use Unix-style `-Wl,-export_dynamic`.
    // On Windows the `.dll` import/export contract is symbol-driven via
    // `.def` files, `__declspec(dllexport)` annotations, or the
    // `/EXPORT:` linker switch.
    //
    // For lbug extensions to resolve symbols back into the host binary on
    // Windows we need `/EXPORT:` or building lbug as an in-tree static
    // library. Both options are scope larger than v1.5 PR3 — the spec
    // (AS-006) requires the arm to *exist* and document the choice; the
    // empirical "extensions actually load on Windows" verification is
    // deferred until Windows CI lands (PR1b + S-002 AS-009 sequencing).
    //
    // For now we emit `/INCREMENTAL:NO` so we get a stable build artifact;
    // the dynamic-export wiring is documented as a known gap for the
    // first user who actually tries lbug extensions on Windows. When that
    // user arrives, swap this for an explicit `/EXPORT:` list (or move
    // lbug to a static lib build).
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-arg=/INCREMENTAL:NO");
        // Document the gap inline so future maintainers don't accidentally
        // ship a Windows binary thinking it has dynamic-export parity.
        println!(
            "cargo:warning=v1.5 PR3 AS-006: Windows linker arm present but lbug \
             extension dynamic-export is unverified. See \
             docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-foundation.md \
             AS-006 known-gap clause."
        );
    }
}
