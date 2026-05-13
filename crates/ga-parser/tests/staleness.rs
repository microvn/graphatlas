//! S-005 AS-013 — staleness checker with 500ms per-process cache + exotic-FS
//! degradation.

use ga_parser::staleness::{StaleResult, StalenessChecker, STALE_CACHE_TTL};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

#[test]
fn stale_cache_ttl_is_500ms_per_spec() {
    // AS-013: "Cache result for T=500ms within MCP process".
    assert_eq!(STALE_CACHE_TTL, std::time::Duration::from_millis(500));
}

#[test]
fn fresh_cache_returns_not_stale() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let hash = checker.compute_now().unwrap();
    let result = checker.check(&hash).unwrap();
    assert!(!result.stale, "same hash → not stale");
    assert_eq!(result.current_hash, hash);
    assert!(!result.degraded);
}

#[test]
fn empty_stored_hash_means_first_run_stale() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    // First-run: metadata.indexed_root_hash == empty string/all-zeros.
    let empty = [0u8; 32];
    let result = checker.check(&empty).unwrap();
    assert!(
        result.stale,
        "first run (empty stored hash) must report stale"
    );
}

#[test]
fn edited_file_triggers_stale() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("sub/app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let baseline = checker.compute_now().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(&tmp.path().join("sub/new.py"), "");

    // Invalidate the 500ms cache.
    checker.invalidate_cache();
    let result = checker.check(&baseline).unwrap();
    assert!(result.stale, "new file in sub/ → stale");
    assert_ne!(result.current_hash, baseline);
}

#[test]
fn cache_amortizes_repeat_calls_within_ttl() {
    // Calls within 500ms must return the same cached hash without
    // re-walking the tree. We test this by reading the internal counter
    // via a cheap proxy: call check() 3× and confirm compute_count == 1.
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let hash = checker.compute_now().unwrap();
    // compute_now already filled the cache. Three check() calls within TTL
    // should not increment the compute counter.
    let before = checker.compute_count();
    for _ in 0..3 {
        let _ = checker.check(&hash).unwrap();
    }
    assert_eq!(checker.compute_count(), before, "cache must not recompute");
}

#[test]
fn cache_expires_after_ttl() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let hash = checker.compute_now().unwrap();
    let before = checker.compute_count();

    // Force cache invalidation rather than waiting 500ms in a unit test.
    checker.invalidate_cache();
    let _ = checker.check(&hash).unwrap();
    assert!(
        checker.compute_count() > before,
        "expired cache must recompute"
    );
}

#[test]
fn degraded_flag_off_for_normal_filesystem() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let hash = checker.compute_now().unwrap();
    let result = checker.check(&hash).unwrap();
    assert!(!result.degraded, "regular tempdir is not exotic FS");
}

#[test]
fn stale_result_fields_populated() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("a.py"), "");
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let hash = checker.compute_now().unwrap();
    let r: StaleResult = checker.check(&hash).unwrap();
    // Type check: ensure all four fields are exposed.
    let _ = r.stale;
    let _ = r.current_hash;
    let _ = r.indexed_root_hash;
    let _ = r.degraded;
}
