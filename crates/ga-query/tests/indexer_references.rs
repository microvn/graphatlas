//! Foundation-C15 — indexer populates REFERENCES rel table.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

#[test]
fn ts_dispatch_map_emits_references_edges() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handlers.ts"),
        "export function handleUsers() { return 'u'; }\nexport function handlePosts() { return 'p'; }\n",
    );
    write(
        &repo.join("routes.ts"),
        "import { handleUsers, handlePosts } from './handlers';\nexport function setup() {\n  const routes = { '/api/users': handleUsers, '/api/posts': handlePosts };\n  return routes;\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.references_edges >= 2,
        "expect ≥2 REFERENCES edges for 2 map values, got {}",
        stats.references_edges
    );

    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (caller:Symbol)-[:REFERENCES]->(target:Symbol) \
             WHERE target.name = 'handleUsers' RETURN caller.name",
        )
        .unwrap();
    let mut callers: Vec<String> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            callers.push(s);
        }
    }
    assert!(
        callers.contains(&"setup".to_string()),
        "setup should reference handleUsers; got {:?}",
        callers
    );
}

#[test]
fn python_dict_pair_emits_references_edges() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def handle_a(): pass\ndef handle_b(): pass\n\ndef wire():\n    handlers = {'a': handle_a, 'b': handle_b}\n    return handlers\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.references_edges >= 2,
        "got {}",
        stats.references_edges
    );
}

#[test]
fn no_references_emits_zero() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("m.py"), "def a(): pass\ndef b():\n    a()\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.references_edges, 0);
}

#[test]
fn references_to_unknown_target_dropped() {
    // handler lives outside repo — reference target unresolved → edge drop.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.ts"),
        "export function setup() {\n  const map = { a: externalThing };\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.references_edges, 0);
}
