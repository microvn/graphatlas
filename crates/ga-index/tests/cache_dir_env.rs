//! S-003 Foundation-C8: `GRAPHATLAS_CACHE_DIR` validation.
//!
//! Must reject:
//!  - sensitive prefixes: `~/.ssh`, `~/.gnupg`, `~/.config/gh`, `/etc`, `/var`
//!  - existing directories with mode more permissive than 0700
//!  - (foreign owner check is Unix-only, skipped on non-Unix)
//!
//! Must accept:
//!  - nonexistent target path (created with 0700)
//!  - existing dir with mode exactly 0700 owned by current user

use ga_index::cache::validate_cache_dir_override;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn nonexistent_path_is_accepted() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("fresh-subdir");
    validate_cache_dir_override(&target).expect("nonexistent path should be accepted");
}

#[test]
fn rejects_ssh_prefix() {
    // We don't need to actually touch ~/.ssh — the function rejects purely
    // based on path string. Keep the test hermetic.
    let paths = [
        Path::new("/home/user/.ssh"),
        Path::new("/home/user/.ssh/cache"),
        Path::new("/Users/alice/.ssh"),
        Path::new("/root/.ssh/subdir/x"),
    ];
    for p in paths {
        let err = validate_cache_dir_override(p)
            .err()
            .unwrap_or_else(|| panic!("should reject {p:?}"));
        let s = format!("{err}");
        assert!(
            s.contains(".ssh") || s.contains("unsafe"),
            "err for {p:?}: {s}"
        );
    }
}

#[test]
fn rejects_gnupg_prefix() {
    let err = validate_cache_dir_override(Path::new("/home/user/.gnupg/cache"))
        .expect_err("should reject");
    assert!(format!("{err}").contains(".gnupg") || format!("{err}").contains("unsafe"));
}

#[test]
fn rejects_gh_config_prefix() {
    let err =
        validate_cache_dir_override(Path::new("/Users/x/.config/gh")).expect_err("should reject");
    assert!(format!("{err}").contains("gh") || format!("{err}").contains("unsafe"));
}

#[test]
fn rejects_etc_prefix() {
    let err = validate_cache_dir_override(Path::new("/etc/graphatlas")).expect_err("should reject");
    assert!(format!("{err}").contains("/etc") || format!("{err}").contains("unsafe"));
}

#[test]
fn rejects_var_system_prefix() {
    // /var/lib and /var/log are system-managed; reject.
    for p in ["/var/lib/ga", "/var/log/ga", "/var"] {
        let err = validate_cache_dir_override(Path::new(p))
            .err()
            .unwrap_or_else(|| panic!("should reject {p}"));
        assert!(
            format!("{err}").contains("/var") || format!("{err}").contains("unsafe"),
            "err for {p}: {err}"
        );
    }
}

#[test]
fn accepts_macos_tmpdir_under_var_folders() {
    // macOS $TMPDIR is /var/folders/<hash>/T/... — must NOT be rejected.
    // (Needed so tests and default-cache fallbacks work on macOS.)
    validate_cache_dir_override(Path::new("/var/folders/xx/T/ga-cache"))
        .expect("allow /var/folders");
    validate_cache_dir_override(Path::new("/var/tmp/ga-cache")).expect("allow /var/tmp");
}

#[cfg(unix)]
#[test]
fn rejects_existing_dir_more_permissive_than_0700() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("too-permissive");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

    let err = validate_cache_dir_override(&target).expect_err("should reject");
    let s = format!("{err}");
    assert!(s.contains("0700") || s.contains("unsafe"), "err: {s}");
}

#[cfg(unix)]
#[test]
fn accepts_existing_0700_dir() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("ok");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();

    validate_cache_dir_override(&target).expect("0700 dir should be accepted");
}

#[test]
fn store_open_with_root_validates_the_root() {
    // End-to-end: Store::open_with_root(/etc/something, ...) must refuse
    // because /etc/ is on the reject list.
    let err =
        ga_index::Store::open_with_root(Path::new("/etc/graphatlas-spoof"), Path::new("/work/x"))
            .err()
            .expect("Store::open_with_root must enforce C8 validation");
    let s = format!("{err}");
    assert!(s.contains("/etc") || s.contains("unsafe"), "err: {s}");
}
