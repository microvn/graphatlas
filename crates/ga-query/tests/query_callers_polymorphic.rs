//! Tools S-001 cluster E — AS-003 polymorphic confidence.
//!
//! Confidence field on each CallerEntry:
//!   - 1.0 when the callee name has exactly one definition in the graph,
//!     OR when the caller's callee file matches the caller's `file` filter.
//!   - 0.6 when the name is multiply defined and the caller's callee lives
//!     in a different file from the filter (or when there is no filter).

use ga_index::Store;
use ga_query::{callers, indexer::build_index};
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
fn single_definition_yields_confidence_one() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("m.py"),
        "def target(): pass\n\ndef caller():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert_eq!(resp.callers.len(), 1);
    assert!(
        (resp.callers[0].confidence - 1.0).abs() < 1e-6,
        "confidence={}",
        resp.callers[0].confidence
    );
}

#[test]
fn multi_def_no_filter_yields_ambiguous_confidence() {
    // "target" defined in both a.py and b.py, each with its own caller.
    // Without a file filter the query cannot prove which def any caller
    // intended → confidence 0.6 for every entry.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\ndef caller_a():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\ndef caller_b():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", None).unwrap();
    assert_eq!(resp.callers.len(), 2);
    for c in &resp.callers {
        assert!(
            (c.confidence - 0.6).abs() < 1e-6,
            "expected 0.6 for {}, got {}",
            c.symbol,
            c.confidence
        );
    }
}

#[test]
fn file_filter_splits_exact_and_polymorphic() {
    // Filter to a.py → caller_a (in a.py, calls a.py::target) = 1.0
    //                  caller_b (in b.py, calls b.py::target) = 0.6 (polymorphic)
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\ndef caller_a():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\ndef caller_b():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", Some("a.py")).unwrap();
    assert_eq!(resp.callers.len(), 2, "{:?}", resp.callers);
    let (exact, poly): (Vec<_>, Vec<_>) = resp
        .callers
        .iter()
        .partition(|c| (c.confidence - 1.0).abs() < 1e-6);
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].symbol, "caller_a");
    assert_eq!(poly.len(), 1);
    assert_eq!(poly[0].symbol, "caller_b");
    assert!((poly[0].confidence - 0.6).abs() < 1e-6);
}

#[test]
fn other_file_callers_included_as_polymorphic() {
    // Ensures polymorphic expansion brings in callers from OTHER files that
    // reference a same-named def, not only the callers within the filter.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def target(): pass\ndef only_exact():\n    target()\n",
    );
    write(
        &repo.join("b.py"),
        "def target(): pass\ndef only_poly_1():\n    target()\n\ndef only_poly_2():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = callers(&store, "target", Some("a.py")).unwrap();
    let names: Vec<String> = resp.callers.iter().map(|c| c.symbol.clone()).collect();
    assert!(names.contains(&"only_exact".to_string()));
    assert!(names.contains(&"only_poly_1".to_string()));
    assert!(names.contains(&"only_poly_2".to_string()));
    // Polymorphic callers must each carry confidence 0.6.
    for c in resp.callers.iter().filter(|c| c.symbol != "only_exact") {
        assert!((c.confidence - 0.6).abs() < 1e-6, "{c:?}");
    }
}
