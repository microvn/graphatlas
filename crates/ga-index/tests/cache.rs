//! S-003 AS-029 cache path derivation + Unix permission enforcement.
//! Foundation-C12: `~/.graphatlas/<repo-name>-<6-hex-sha256-of-repo_root>/`.
//! Foundation-C8: dir 0700, files 0600; refuse open if more permissive.

use ga_index::cache::{short_hash, CacheLayout};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn short_hash_is_6_hex_of_sha256() {
    // Deterministic: same input → same output; exactly 6 hex chars.
    let h1 = short_hash("/work/client1/billing-api");
    let h2 = short_hash("/work/client1/billing-api");
    assert_eq!(h1, h2, "sha256 must be deterministic");
    assert_eq!(h1.len(), 6, "short hash must be 6 chars");
    assert!(
        h1.chars().all(|c| c.is_ascii_hexdigit()),
        "must be hex: {h1}"
    );

    // Different input → different hash (not a collision proof, just a smoke).
    let h_other = short_hash("/work/client2/billing-api");
    assert_ne!(h1, h_other);
}

#[test]
fn layout_includes_repo_name_and_hash() {
    // AS-028 sample: billing-api-9f2d1e under cache root.
    let root = Path::new("/tmp/fake-home/.graphatlas");
    let layout = CacheLayout::for_repo(root, Path::new("/work/client1/billing-api"));
    assert_eq!(layout.repo_name(), "billing-api");
    assert_eq!(
        layout.dir_name(),
        format!("billing-api-{}", short_hash("/work/client1/billing-api"))
    );
    assert_eq!(layout.dir(), root.join(layout.dir_name()));
    assert_eq!(layout.graph_db(), layout.dir().join("graph.db"));
    assert_eq!(layout.metadata_json(), layout.dir().join("metadata.json"));
    assert_eq!(layout.lock_pid(), layout.dir().join("lock.pid"));
}

#[test]
fn layout_handles_trailing_slash_same_hash() {
    // /work/billing-api and /work/billing-api/ should resolve to same cache.
    let root = Path::new("/tmp/fake/.graphatlas");
    let a = CacheLayout::for_repo(root, Path::new("/work/billing-api"));
    let b = CacheLayout::for_repo(root, Path::new("/work/billing-api/"));
    assert_eq!(a.dir_name(), b.dir_name());
}

#[test]
fn layout_empty_repo_name_falls_back_to_root() {
    // Edge case: repo_root = "/" → repo_name defaults to "root".
    let root = Path::new("/tmp/fake/.graphatlas");
    let layout = CacheLayout::for_repo(root, Path::new("/"));
    assert_eq!(layout.repo_name(), "root");
}

#[cfg(unix)]
#[test]
fn ensure_dir_creates_with_0700() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas");
    let layout = CacheLayout::for_repo(&root, Path::new("/work/example"));
    layout.ensure_dir().expect("create dir");
    let meta = std::fs::metadata(layout.dir()).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "expected 0700, got {mode:o}");
}

#[cfg(unix)]
#[test]
fn ensure_dir_rejects_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(".graphatlas");
    let layout = CacheLayout::for_repo(&root, Path::new("/work/example"));
    // Pre-create with unsafe 0755 mode.
    std::fs::create_dir_all(layout.dir()).unwrap();
    std::fs::set_permissions(layout.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = layout.ensure_dir().expect_err("should refuse 0755");
    let s = format!("{err}");
    assert!(s.contains("unsafe"), "err text: {s}");
    assert!(s.contains("0700") || s.contains("chmod"), "err text: {s}");
}

#[cfg(unix)]
#[test]
fn store_open_sets_graph_db_mode_0600() {
    // AS-029: graph.db must be created/retained at mode 0600 after Store::open.
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo = Path::new("/work/perms-smoke");
    let store = ga_index::Store::open_with_root(&cache_root, repo).unwrap();
    let db_path = store.layout().graph_db();
    let meta = std::fs::metadata(&db_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "graph.db mode must be 0600, got 0{mode:o}");
}

#[cfg(unix)]
#[test]
fn store_open_refuses_permissive_graph_db_on_reopen() {
    // AS-029 error path on graph.db specifically (previously only covered metadata.json).
    //
    // v1.5 PR2: repo path must exist on disk (commit_in_place now calls
    // ga_parser::merkle::compute_root_hash to populate indexed_root_hash —
    // foundation S-001 AS-001). Use a TempDir subdir as a real fixture
    // repo to keep this test's AS-029 intent intact.
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let repo_dir = tmp.path().join("repo").join("perms-reopen");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# fixture\n").unwrap();
    let repo = repo_dir.as_path();
    {
        let s = ga_index::Store::open_with_root(&cache_root, repo).unwrap();
        s.commit().unwrap();
    }
    // Attacker/accident sets graph.db to 0644.
    let db_path = ga_index::cache::CacheLayout::for_repo(&cache_root, repo).graph_db();
    std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let err = ga_index::Store::open_with_root(&cache_root, repo)
        .err()
        .expect("must refuse permissive graph.db");
    let s = format!("{err}");
    assert!(s.contains("unsafe permissions"), "err: {s}");
    assert!(s.contains("chmod 0600") || s.contains("0o600"), "err: {s}");
}

#[cfg(unix)]
#[test]
fn ensure_file_mode_enforces_0600() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("metadata.json");
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let err = ga_index::cache::verify_file_perms(&path).expect_err("should refuse 0644");
    assert!(format!("{err}").contains("unsafe permissions"));

    // Fix to 0600 → should accept.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    ga_index::cache::verify_file_perms(&path).expect("0600 accepted");
}
