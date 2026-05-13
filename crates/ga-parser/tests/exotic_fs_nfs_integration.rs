//! v1.5 PR3 foundation / staleness sub-spec — degraded FS integration tests.
//!
//! Closes the StPR5.AS-010 carve-out ("Degraded FS → meta.stale_check_degraded"):
//! the code-side handler logic was covered in PR5 unit tests, but a
//! real-mount integration test was deferred because we lacked an NFS/FUSE
//! fixture. This file holds `#[ignore]`-gated tests that run against a
//! pre-mounted exotic filesystem so the empirical guarantee can be
//! checked locally and in CI runners that provide such a mount.
//!
//! **How to run** (local):
//!
//! ```bash
//! # Linux NFS loopback fixture:
//! sudo apt-get install -y nfs-kernel-server
//! sudo mkdir -p /srv/ga-nfs && sudo chown $USER /srv/ga-nfs
//! echo "/srv/ga-nfs 127.0.0.1(rw,sync,no_subtree_check,no_root_squash)" \
//!   | sudo tee /etc/exports
//! sudo exportfs -ra && sudo systemctl restart nfs-kernel-server
//! mkdir -p /tmp/ga-nfs-mount
//! sudo mount -t nfs 127.0.0.1:/srv/ga-nfs /tmp/ga-nfs-mount
//! GA_EXOTIC_FS_MOUNT=/tmp/ga-nfs-mount \
//!   cargo test -p ga-parser --test exotic_fs_nfs_integration -- --ignored
//! ```
//!
//! Without the env var, all `#[ignore]`-gated tests no-op so a regular
//! `cargo test` run never blocks on infra that isn't available.

use ga_parser::staleness::StalenessChecker;
use std::path::PathBuf;

fn nfs_mount_root() -> Option<PathBuf> {
    let p = std::env::var("GA_EXOTIC_FS_MOUNT").ok()?;
    let pb = PathBuf::from(p);
    if pb.exists() {
        Some(pb)
    } else {
        eprintln!(
            "GA_EXOTIC_FS_MOUNT={} not found — skipping",
            pb.display()
        );
        None
    }
}

/// StPR5.AS-010 — when the staleness checker runs against a path on an
/// exotic filesystem, the result must surface `degraded: true` so the
/// MCP gate can propagate `meta.stale_check_degraded: true` to tool
/// responses (PR5 wire).
///
/// `#[ignore]` because it requires a real NFS/FUSE mount at the path
/// named by `$GA_EXOTIC_FS_MOUNT`. Run with `--ignored` after exporting
/// that env var per the module-level docs above.
#[test]
#[ignore = "needs NFS/FUSE mount at $GA_EXOTIC_FS_MOUNT — see module docs"]
fn st_pr5_as_010_exotic_fs_mount_sets_degraded_true() {
    let mount = match nfs_mount_root() {
        Some(p) => p,
        None => {
            eprintln!("skip: $GA_EXOTIC_FS_MOUNT not set");
            return;
        }
    };
    // Seed a minimal repo on the exotic mount so the Merkle scan has
    // something to walk.
    let repo = mount.join("ga_staleness_probe");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# nfs probe\n").unwrap();
    let checker = StalenessChecker::new(repo);

    // First compute populates the cache + samples filesystem type.
    let result = checker.check(&[0u8; 32]).expect("staleness check on NFS");
    assert!(
        result.degraded,
        "AS-010: NFS mount must set degraded=true so MCP surfaces meta.stale_check_degraded; \
         current arms (linux/macos/windows) may need a statfs-magic update — \
         see crates/ga-parser/src/staleness.rs::is_exotic_filesystem"
    );
}

/// Local-FS regression — confirms a regular tempdir is NOT degraded.
/// This one runs under normal `cargo test` (no `#[ignore]`) so we keep a
/// fast guard against an over-eager exotic-FS classifier accidentally
/// flagging local mounts.
#[test]
fn local_tempdir_is_not_degraded_regression() {
    let tmp = tempfile::TempDir::new().unwrap();
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let result = checker.check(&[0u8; 32]).expect("staleness check on local fs");
    assert!(
        !result.degraded,
        "regression: local tempdir must NOT be classified exotic"
    );
}
