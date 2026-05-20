//! S-004 follow-up — `LbugDataSource` end-to-end smoke against a real
//! ga-index Store. Reindexes a tiny Rust fixture, then exercises every
//! trait method through `LbugDataSource`.
//!
//! Consolidated into a single `#[test]` because reindex flow reads
//! `GRAPHATLAS_CACHE_DIR` at runtime — parallel tests would trample it.

use std::path::PathBuf;

use ga_server::data::{DataError, ProjectDataSource};
use ga_server::LbugDataSource;
use tempfile::TempDir;

#[allow(dead_code)]
fn _silence_unused_pathbuf(_p: &PathBuf) {}

fn fixture_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        b"pub fn alpha() -> &'static str {\n    beta()\n}\n\npub fn beta() -> &'static str {\n    \"hi\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    tmp
}

#[test]
fn lbug_source_full_smoke_against_real_reindex() {
    let cache = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("GRAPHATLAS_CACHE_DIR", cache.path());

    let repo = fixture_repo();
    let repo_path = repo.path().canonicalize().unwrap();
    graphatlas::cmd_reindex::do_reindex(&repo_path, false).expect("reindex must succeed");

    let cache_root: PathBuf = cache.path().to_path_buf();
    let slug = std::fs::read_dir(&cache_root)
        .unwrap()
        .flatten()
        .find_map(|e| {
            let p = e.path();
            if !p.is_dir() {
                return None;
            }
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|s| s.rsplit('-').next())
                .map(String::from)
        })
        .expect("at least one cache dir after reindex");

    let src = LbugDataSource::new(cache_root.clone());

    // 1. callers — `beta` is called by `alpha`.
    let page = src.callers(&slug, "beta", 0, 50).expect("callers");
    assert!(page.total >= 1, "expected ≥1 caller of beta, got {}", page.total);
    assert!(page.entries.iter().any(|e| e.name == "alpha"));

    // 2. callees — `alpha` calls `beta`.
    let page = src.callees(&slug, "alpha", 0, 50).expect("callees");
    assert!(page.total >= 1);
    assert!(page.entries.iter().any(|e| e.name == "beta"));

    // 3. file_summary — both symbols present.
    let summary = src.file_summary(&slug, "src/lib.rs").expect("file_summary");
    let names: Vec<_> = summary.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"alpha"), "names: {:?}", names);
    assert!(names.contains(&"beta"), "names: {:?}", names);

    // 4. symbol_detail — rendered signature non-empty.
    let detail = src.symbol_detail(&slug, "alpha").expect("symbol_detail");
    assert_eq!(detail.name, "alpha");
    assert!(detail.rendered_signature.starts_with("alpha("));

    // 5. graph_dump — both nodes present.
    let g = src.graph_dump(&slug, None, 2).expect("graph_dump");
    assert!(g.total_node_count >= 2);
    assert!(g.nodes.iter().any(|n| n.name == "alpha"));
    assert!(g.nodes.iter().any(|n| n.name == "beta"));

    // 6. Error paths.
    assert_eq!(
        src.callers("nope0001", "alpha", 0, 50).unwrap_err(),
        DataError::ProjectNotFound
    );
    assert_eq!(
        src.symbol_detail(&slug, "ghost_symbol").unwrap_err(),
        DataError::SymbolNotFound
    );
    assert_eq!(
        src.file_summary(&slug, "src/__never__.rs").unwrap_err(),
        DataError::FileNotFound
    );

    std::env::remove_var("GRAPHATLAS_CACHE_DIR");
}
