//! ga-ui Spec A S-003 follow-up — end-to-end verification that
//! `do_reindex` populates the new sidecar fields on metadata.json.
//!
//! Boots `do_reindex` in-process against a tiny tempdir fixture so the
//! whole pipeline runs (Store::reindex_in_place + indexer::build_index
//! + Metadata::commit_in_place + the new counts/health post-commit).

use ga_core::IndexState;
use ga_index::metadata::Metadata;
use std::path::PathBuf;
use tempfile::TempDir;

/// Build a minimal Rust fixture: one .rs file with one function. The
/// indexer only needs *some* symbols to count — content doesn't matter
/// for the sidecar metadata-shape assertion.
fn fixture_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        b"pub fn hello() -> &'static str { \"hi\" }\n",
    )
    .unwrap();
    // Minimal Cargo.toml so the indexer's Rust path detector lights up.
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    tmp
}

fn locate_metadata(cache_root: &std::path::Path, repo_root: &std::path::Path) -> PathBuf {
    let target = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let entries = std::fs::read_dir(cache_root).unwrap();
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(target) {
            return p.join("metadata.json");
        }
    }
    panic!(
        "no cache dir matched repo basename {} under {}",
        target,
        cache_root.display()
    );
}

#[test]
fn reindex_populates_index_counts_and_health_summary_in_metadata() {
    let cache = tempfile::tempdir().unwrap();
    // Foundation H-2: cache root must be 0700.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            cache.path(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    // Cache root override — Store::open reads GRAPHATLAS_CACHE_DIR.
    std::env::set_var("GRAPHATLAS_CACHE_DIR", cache.path());

    let repo = fixture_repo();
    let repo_path = repo.path().canonicalize().unwrap();

    graphatlas::cmd_reindex::do_reindex(&repo_path, false).expect("reindex should succeed");

    let md_path = locate_metadata(cache.path(), &repo_path);
    let bytes = std::fs::read(&md_path).expect("read metadata.json");
    let md: Metadata =
        serde_json::from_slice(&bytes).expect("metadata.json parses after S-003 migration");

    assert!(matches!(md.index_state, IndexState::Complete));

    // index_counts must be populated.
    let counts = md.index_counts.as_ref().expect(
        "S-003 follow-up: reindex must persist index_counts; got None",
    );
    assert!(
        counts.last_index_duration_ms > 0,
        "duration must be non-zero, got {}",
        counts.last_index_duration_ms
    );
    assert!(
        counts.db_size_bytes > 0,
        "cache dir should have on-disk bytes after reindex"
    );
    // Tiny fixture — at least 1 file should land in the graph.
    assert!(
        counts.file_count >= 1,
        "expected ≥1 file indexed, got {}",
        counts.file_count
    );

    // health_summary must be populated (values may be 0 for tiny fixture).
    let health = md.health_summary.as_ref().expect(
        "S-003 follow-up: reindex must persist health_summary; got None",
    );
    assert!(
        health.computed_at_unix > 0,
        "computed_at_unix should be a real epoch second"
    );

    std::env::remove_var("GRAPHATLAS_CACHE_DIR");
}
