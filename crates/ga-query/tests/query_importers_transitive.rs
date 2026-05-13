//! Tools S-003 cluster C — AS-007 transitive importers via re-export.
//! Chain rule: src→A→B→…→dst. First hop (src→A) any kind; subsequent hops
//! must have re_export=true. Depth cap = 3 hops from src to dst.

use ga_index::Store;
use ga_query::{importers, indexer::build_index};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn two_hop_transitive_importer_surfaces_with_via() {
    // baz.ts imports foo.ts (normal). foo.ts re-exports from bar.ts.
    // ga_importers('bar.ts') should include baz.ts with via='foo.ts' and
    // re_export=true — the AS-007 literal example.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("bar.ts"), "export function b() {}\n");
    write(&repo.join("foo.ts"), "export * from './bar';\n");
    write(&repo.join("baz.ts"), "import { b } from './foo';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "bar.ts").unwrap();
    let baz = resp
        .importers
        .iter()
        .find(|e| e.path == "baz.ts")
        .unwrap_or_else(|| {
            panic!(
                "baz.ts expected as transitive importer: {:?}",
                resp.importers
            )
        });
    assert!(
        baz.re_export,
        "transitive entry must flag re_export: {baz:?}"
    );
    assert_eq!(baz.via.as_deref(), Some("foo.ts"), "via mismatch: {baz:?}");
}

#[test]
fn three_hop_transitive_importer_included() {
    // src → m1 → m2 → dst. m1→m2 and m2→dst are re-exports.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("dst.ts"), "export function d() {}\n");
    write(&repo.join("m2.ts"), "export * from './dst';\n");
    write(&repo.join("m1.ts"), "export * from './m2';\n");
    write(&repo.join("src.ts"), "import { d } from './m1';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "dst.ts").unwrap();
    let src = resp
        .importers
        .iter()
        .find(|e| e.path == "src.ts")
        .unwrap_or_else(|| panic!("src.ts expected at depth 3: {:?}", resp.importers));
    assert!(src.re_export);
    assert_eq!(src.via.as_deref(), Some("m1.ts"));
}

#[test]
fn four_hop_chain_excluded() {
    // src → m1 → m2 → m3 → dst (4 hops). Depth cap is 3, so src must NOT
    // surface as an importer of dst.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("dst.ts"), "export function d() {}\n");
    write(&repo.join("m3.ts"), "export * from './dst';\n");
    write(&repo.join("m2.ts"), "export * from './m3';\n");
    write(&repo.join("m1.ts"), "export * from './m2';\n");
    write(&repo.join("src.ts"), "import { d } from './m1';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "dst.ts").unwrap();
    assert!(
        !resp.importers.iter().any(|e| e.path == "src.ts"),
        "src.ts at 4 hops should be excluded: {:?}",
        resp.importers
    );
}

#[test]
fn non_reexport_intermediate_does_not_bubble() {
    // src imports foo. foo imports bar as a NORMAL import (not re-export).
    // ga_importers('bar.ts') must NOT include src — the chain breaks.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("bar.ts"), "export function b() {}\n");
    write(
        &repo.join("foo.ts"),
        "import { b } from './bar';\nexport function f() {}\n",
    );
    write(&repo.join("src.ts"), "import { f } from './foo';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "bar.ts").unwrap();
    let paths: Vec<&str> = resp.importers.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"foo.ts"),
        "foo.ts should be a direct importer: {paths:?}"
    );
    assert!(
        !paths.contains(&"src.ts"),
        "src.ts must NOT surface — foo's import is not re-export: {paths:?}"
    );
}

#[test]
fn direct_wins_over_transitive_on_dedup() {
    // src both directly imports bar AND reaches bar transitively via foo.
    // Result should have src exactly once as a DIRECT importer (re_export=false).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("bar.ts"), "export function b() {}\n");
    write(&repo.join("foo.ts"), "export * from './bar';\n");
    write(
        &repo.join("src.ts"),
        "import { b } from './bar';\nimport { b as b2 } from './foo';\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "bar.ts").unwrap();
    let src_entries: Vec<_> = resp
        .importers
        .iter()
        .filter(|e| e.path == "src.ts")
        .collect();
    assert_eq!(src_entries.len(), 1, "{src_entries:?}");
    assert!(
        !src_entries[0].re_export,
        "direct entry wins: {src_entries:?}"
    );
    assert!(src_entries[0].via.is_none());
}

#[test]
fn reexport_cycle_does_not_surface_self() {
    // a.ts re-exports b.ts, b.ts re-exports a.ts. Pathological but possible.
    // Query ga_importers('a.ts'). b.ts is a direct importer. a.ts itself
    // must NOT appear in its own importers (self-loop filter).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.ts"), "export * from './b';\n");
    write(&repo.join("b.ts"), "export * from './a';\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = importers(&store, "a.ts").unwrap();
    let paths: Vec<&str> = resp.importers.iter().map(|e| e.path.as_str()).collect();
    assert!(!paths.contains(&"a.ts"), "self must not appear: {paths:?}");
}
