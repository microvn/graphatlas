//! v1.4 S-002 — IMPORTS_NAMED.is_type_only attribute (TS dead-code accuracy).
//!
//! Spec: graphatlas-v1.4-data-model.md S-002, AS-015 / AS-016 / AS-017.
//! AS-018 (dead_code TS-aware classification with framework-route
//! exemption) lives in a separate test surface — see
//! v1_4_s_002_dead_code_type_only.rs.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn fetch_imports_named(store: &Store, src_file: &str) -> Vec<(String, bool)> {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (f:File {{path: '{src_file}'}})-[r:IMPORTS_NAMED]->(t:Symbol) \
         RETURN t.name, r.is_type_only ORDER BY t.name"
    );
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if cols.len() < 2 {
            continue;
        }
        let n = match &cols[0] {
            lbug::Value::String(s) => s.clone(),
            _ => continue,
        };
        let t = matches!(&cols[1], lbug::Value::Bool(true));
        out.push((n, t));
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// AS-015 — TS `import type` and `import` distinguished per-name
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ts_import_type_and_import_distinguished_per_row() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("mod.ts"),
        "export class Foo {}\nexport function Bar(): void {}\n",
    );
    write(
        &repo.join("app.ts"),
        "import type { Foo } from './mod';\nimport { Bar } from './mod';\n\
         export function use(): void { Bar(); }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let rows = fetch_imports_named(&store, "app.ts");
    assert!(
        rows.contains(&("Foo".to_string(), true)),
        "AS-015: app.ts must have IMPORTS_NAMED(Foo, is_type_only=true); got {rows:?}"
    );
    assert!(
        rows.contains(&("Bar".to_string(), false)),
        "AS-015: app.ts must have IMPORTS_NAMED(Bar, is_type_only=false); got {rows:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-016 — Mixed `import { Foo, type Bar }` per-name flag
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ts_mixed_import_per_name_flag() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("mod.ts"),
        "export class Bar {}\nexport function Foo(): void {}\n",
    );
    write(
        &repo.join("app.ts"),
        "import { Foo, type Bar } from './mod';\n\
         export function use(): void { Foo(); }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let rows = fetch_imports_named(&store, "app.ts");
    assert!(
        rows.contains(&("Foo".to_string(), false)),
        "AS-016: mixed import — Foo (no `type` keyword) is_type_only=false; got {rows:?}"
    );
    assert!(
        rows.contains(&("Bar".to_string(), true)),
        "AS-016: mixed import — Bar (with `type` keyword) is_type_only=true; got {rows:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-017 — TS `export type { Foo } from './mod'` re-export propagates flag
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ts_export_type_reexport_propagates_is_type_only() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("mod.ts"),
        "export class Foo {}\nexport function Bar(): void {}\n",
    );
    write(
        &repo.join("re-exports.ts"),
        "export type { Foo } from './mod';\n\
         export { Bar } from './mod';\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let rows = fetch_imports_named(&store, "re-exports.ts");
    assert!(
        rows.contains(&("Foo".to_string(), true)),
        "AS-017: `export type {{ Foo }}` re-export must propagate is_type_only=true; got {rows:?}"
    );
    assert!(
        rows.contains(&("Bar".to_string(), false)),
        "AS-017: `export {{ Bar }}` re-export must be is_type_only=false; got {rows:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-018 — dead_code TS-aware classification with framework-route exemption
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn as_018_framework_routed_files_exempt_from_dead_code() {
    use ga_query::dead_code::{dead_code, DeadCodeRequest};
    // Case-B per spec AS-018: app/api/users/route.ts (Next.js App Router
    // pattern) exports GET handler with no static IMPORTS_NAMED inbound.
    // Without the framework-route exemption, GET would be flagged dead.
    // With exemption, GET MUST NOT appear in dead-code result.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    // Next.js App Router pattern: app/api/users/route.ts.
    write(
        &repo.join("app/api/users/route.ts"),
        "export function GET(): Response { return new Response(); }\n",
    );
    // Sibling type-only consumer (irrelevant to AS-018 but documents the
    // type-only-only-importer scenario).
    write(
        &repo.join("app/types.ts"),
        "import type { GET } from './api/users/route';\nexport type Handler = typeof GET;\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let dead_files: Vec<&str> = resp.dead.iter().map(|e| e.file.as_str()).collect();

    assert!(
        !dead_files.contains(&"app/api/users/route.ts"),
        "AS-018: app/api/users/route.ts is a Next.js App Router auto-discovered \
         handler — must NOT appear in dead_code result. dead_files={dead_files:?}"
    );
}

#[test]
fn as_018_non_framework_path_still_flagged_dead() {
    use ga_query::dead_code::{dead_code, DeadCodeRequest};
    // Inverse / anti-theatre: a TS file outside framework-route patterns
    // with a function that has zero callers MUST still be flagged dead.
    // Confirms the exemption only fires for actual framework patterns.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("src/lib/orphan.ts"),
        "export function unusedHelper(): void {}\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = dead_code(&store, &DeadCodeRequest::default()).expect("dead_code ok");
    let dead_pairs: Vec<(String, String)> = resp
        .dead
        .iter()
        .map(|e| (e.symbol.clone(), e.file.clone()))
        .collect();

    assert!(
        dead_pairs.contains(&("unusedHelper".to_string(), "src/lib/orphan.ts".to_string())),
        "AS-018 anti-theatre: src/lib/orphan.ts::unusedHelper has no callers and is \
         not framework-routed — MUST be flagged dead. dead_pairs={dead_pairs:?}"
    );
}
