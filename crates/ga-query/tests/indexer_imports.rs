//! Tools S-003 cluster A — indexer writes IMPORTS edges.

use ga_index::Store;
use ga_query::indexer::build_index;
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
fn python_from_import_emits_imports_edge() {
    // a.py imports from utils/format.py → IMPORTS(a.py → utils/format.py).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("utils/format.py"), "def fmt(): pass\n");
    write(&repo.join("a.py"), "from utils.format import fmt\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert!(
        stats.imports_edges >= 1,
        "expected IMPORTS edge, got {stats:?}"
    );

    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (src:File)-[:IMPORTS]->(dst:File) \
             WHERE dst.path = 'utils/format.py' \
             RETURN src.path",
        )
        .unwrap();
    let srcs: Vec<String> = rs
        .into_iter()
        .filter_map(|r| match r.into_iter().next() {
            Some(lbug::Value::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(srcs, vec!["a.py".to_string()], "{srcs:?}");
}

#[test]
fn external_import_produces_no_edge() {
    // `import os` has no os.py in the repo → no IMPORTS edge created.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("a.py"), "import os\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let stats = build_index(&store, &repo).unwrap();
    assert_eq!(stats.imports_edges, 0, "{stats:?}");
}

#[test]
fn imports_reindex_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("b.py"), "def g(): pass\n");
    write(&repo.join("a.py"), "from b import g\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let s1 = build_index(&store, &repo).unwrap();
    let s2 = build_index(&store, &repo).unwrap();
    assert_eq!(s1.imports_edges, s2.imports_edges);
    assert_eq!(s1.imports_edges, 1);
}
