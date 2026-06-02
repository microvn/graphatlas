//! ImpactedFile self-explaining fields: confidence + relation_to_seed +
//! explanation. Universal vocabulary (LLM consumer with no GA spec context
//! must still understand each row). Bench-safe — fields are additive,
//! retriever extracts only `path`, scoring unchanged.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn impacted_file_carries_confidence_one_for_single_def_seed() {
    // Single-def symbol → Tools-C11 says confidence 1.0 unconditional.
    // Self-explaining response must propagate this to ImpactedFile.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("solo.py"), "def unique_fn():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("unique_fn".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let seed = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "solo.py")
        .expect("seed file must surface");
    assert!(
        (seed.confidence - 1.0).abs() < 1e-6,
        "single-def seed must carry confidence=1.0, got {}",
        seed.confidence
    );
}

#[test]
fn impacted_file_carries_polymorphic_confidence_when_same_name_in_multiple_files() {
    // Tools-C11 (b): symbol defined in ≥2 files → polymorphic → 0.6
    // confidence on non-hint definitions. The self-explain layer must
    // surface this so an LLM consumer can prune / rank without reading
    // GA's spec.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def shared():\n    return 1\n");
    write(&repo.join("b.py"), "def shared():\n    return 2\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("shared".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let a = resp.impacted_files.iter().find(|f| f.path == "a.py");
    let b = resp.impacted_files.iter().find(|f| f.path == "b.py");
    assert!(
        a.is_some() && b.is_some(),
        "both defs must surface: {:?}",
        resp.impacted_files
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert!(
        (a.confidence - 1.0).abs() < 1e-6,
        "file-hint match must be 1.0, got {}",
        a.confidence
    );
    assert!(
        (b.confidence - 0.6).abs() < 1e-6,
        "polymorphic non-hint def must be 0.6, got {}",
        b.confidence
    );
}

#[test]
fn impacted_file_relation_uses_universal_vocabulary() {
    // Strings must be self-evident to an LLM with no GA context. No
    // internal taxonomy ("PolymorphicDef", "KinshipViaCallee", etc.).
    // Vocabulary: changed_directly, shares_function_name, calls_seed_directly,
    // called_by_seed_directly, shared_dependency, sibling_in_same_class.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def shared():\n    return 1\n");
    write(&repo.join("b.py"), "def shared():\n    return 2\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("shared".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let a = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "a.py")
        .unwrap();
    let b = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "b.py")
        .unwrap();

    // Hint-matched seed: this is the file the user is about to change.
    assert_eq!(a.relation_to_seed, "changed_directly");
    // Polymorphic sibling: same function name, different file.
    assert_eq!(b.relation_to_seed, "shares_function_name");
}

#[test]
fn impacted_file_explanation_is_non_empty_plain_english() {
    // Each row gets a 1-sentence explanation. Must be non-empty so an
    // LLM client can quote it without bizarre fallbacks.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def shared():\n    return 1\n");
    write(&repo.join("b.py"), "def shared():\n    return 2\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("shared".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .unwrap();

    for f in &resp.impacted_files {
        assert!(
            !f.explanation.is_empty(),
            "every ImpactedFile must carry a non-empty explanation; got empty for {:?}",
            f.path
        );
        assert!(
            f.explanation.len() < 200,
            "explanation must be one short sentence (< 200 chars), got {} chars on {}",
            f.explanation.len(),
            f.path
        );
    }
}

#[test]
fn as_002_guessed_caller_to_ambiguous_seed_is_downgraded_end_to_end() {
    // AS-002: an ambiguous seed (`tgt` defined in 2 files) reached by a bare
    // name-guess call (no import) → that caller is reported at confidence 0.6
    // with the distinct `name_guess` relation. Proves guessed_only is computed
    // from the real CALLS_HEURISTIC edges.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("seed.py"), "def tgt():\n    return 1\n");
    write(&repo.join("other.py"), "def tgt():\n    return 2\n");
    // Bare call, NO import of tgt → resolves repo-wide by name → tier-3 guess.
    write(&repo.join("caller.py"), "def c():\n    return tgt()\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("tgt".into()),
            file: Some("seed.py".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let caller = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "caller.py")
        .expect("guessed caller must surface as impacted");
    assert!(
        (caller.confidence - 0.6).abs() < 1e-6,
        "guessed caller of an ambiguous seed → 0.6, got {}",
        caller.confidence
    );
    assert_eq!(
        caller.relation_to_seed, "name_guess",
        "must use the distinct name_guess token, not shares_function_name"
    );
}

#[test]
fn as_008_guessed_caller_to_unique_seed_keeps_full_confidence_end_to_end() {
    // AS-008: same name-guess edge, but the seed name is UNIQUE → only one
    // possible target → reliable → confidence stays 1.0 (ambiguity gate).
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("seed.py"), "def uniquetgt():\n    return 1\n");
    write(
        &repo.join("caller.py"),
        "def c():\n    return uniquetgt()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("uniquetgt".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let caller = resp
        .impacted_files
        .iter()
        .find(|f| f.path == "caller.py")
        .expect("caller must surface as impacted");
    assert!(
        (caller.confidence - 1.0).abs() < 1e-6,
        "guess at a uniquely-named seed must stay 1.0, got {}",
        caller.confidence
    );
}

#[test]
fn impacted_file_path_membership_unchanged_by_self_explain_fields() {
    // Bench-safety guard: adding fields must not change which files are
    // surfaced. Same input → same set of paths as before this change.
    //
    // Updated 2026-05-22 (CORE-2): use a `file:` hint so the ambiguity-first
    // gate doesn't fire on the multi-def `shared`. Both a.py and b.py still
    // surface — a.py as the seed file, b.py as the homonym (relation
    // `shares_function_name`).
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("a.py"), "def shared():\n    return 1\n");
    write(&repo.join("b.py"), "def shared():\n    return 2\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = impact(
        &store,
        &ImpactRequest {
            symbol: Some("shared".into()),
            file: Some("a.py".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let mut paths: Vec<&str> = resp
        .impacted_files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["a.py", "b.py"]);
}
