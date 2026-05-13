//! Bench P3-C2 — retriever factory used by CLI `--retrievers` flag.

use ga_bench::runner::{build_retrievers, RETRIEVER_NAMES};

#[test]
fn factory_knows_all_registered_retrievers() {
    let names: Vec<&str> = RETRIEVER_NAMES.to_vec();
    // Bench-C3 v1 scope + ripgrep baseline.
    assert!(names.contains(&"ga"));
    assert!(names.contains(&"ripgrep"));
    assert!(names.contains(&"codegraphcontext"));
    assert!(names.contains(&"codebase-memory"));
    assert!(names.contains(&"code-review-graph"));
}

#[test]
fn factory_builds_all_known_names() {
    let tmp = tempfile::TempDir::new().unwrap();
    let set = build_retrievers(
        &[
            "ga",
            "ripgrep",
            "codegraphcontext",
            "codebase-memory",
            "code-review-graph",
        ],
        tmp.path().join(".cache"),
    )
    .expect("all-known names should build");
    let names: Vec<&str> = set.iter().map(|r| r.name()).collect();
    assert_eq!(names.len(), 5);
}

#[test]
fn factory_rejects_unknown_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let res = build_retrievers(
        &["ga", "totally-not-a-real-retriever"],
        tmp.path().join(".cache"),
    );
    let err = match res {
        Ok(_) => panic!("unknown name must error"),
        Err(e) => e,
    };
    let s = format!("{err}");
    assert!(s.contains("totally-not-a-real-retriever"), "{s}");
    assert!(s.contains("ga") || s.contains("supported"), "{s}");
}

#[test]
fn factory_empty_list_is_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let res = build_retrievers(&[], tmp.path().join(".cache"));
    let err = match res {
        Ok(_) => panic!("empty list must error"),
        Err(e) => e,
    };
    let s = format!("{err}");
    assert!(s.contains("retrievers") || s.contains("empty"), "{s}");
}

#[test]
fn factory_preserves_caller_order() {
    // Leaderboard row order should follow user's --retrievers flag order
    // (so `--retrievers ga,cgc` renders ga first even if alphabetical would
    // put cgc first). Factory preserves input order.
    let tmp = tempfile::TempDir::new().unwrap();
    let set = build_retrievers(&["ripgrep", "ga"], tmp.path().join(".cache")).unwrap();
    assert_eq!(set[0].name(), "ripgrep");
    assert_eq!(set[1].name(), "ga");
}
