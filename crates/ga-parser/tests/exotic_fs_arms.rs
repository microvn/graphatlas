//! v1.5 PR3 foundation S-002 AS-008 — `is_exotic_filesystem` cross-platform arms.
//!
//! Behavior verified through the public `StalenessChecker::check` API which
//! surfaces the private `is_exotic_filesystem` decision via the
//! `StaleResult.degraded` field.

use ga_parser::staleness::StalenessChecker;
use tempfile::TempDir;

#[cfg(target_os = "macos")]
#[test]
fn macos_local_tempdir_is_not_degraded() {
    // AS-008 macOS arm: local filesystem under /tmp or /var/folders is
    // NOT flagged exotic by default. The cfg(target_os = "macos") arm
    // returns false so Merkle walks the full N≤32-dir bound.
    let tmp = TempDir::new().unwrap();
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let zero = [0u8; 32];
    let result = checker.check(&zero).expect("check on local tmp");
    assert!(
        !result.degraded,
        "macOS local FS must NOT be degraded by default; got degraded=true on {:?}",
        tmp.path()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_local_tempdir_is_not_degraded() {
    // AS-008 Linux arm: local ext4/btrfs/xfs path not flagged exotic.
    // Real statfs-magic detection (NFS/FUSE/9p/SMB/virtiofs) deferred to
    // PR9 when watcher needs the discrimination.
    let tmp = TempDir::new().unwrap();
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let zero = [0u8; 32];
    let result = checker.check(&zero).expect("check on local tmp");
    assert!(
        !result.degraded,
        "Linux local FS must NOT be degraded by default; got degraded=true on {:?}",
        tmp.path()
    );
}

#[cfg(target_os = "windows")]
#[test]
fn windows_arm_defaults_to_degraded_for_polling_fallback() {
    // AS-008 Windows arm: conservative default. Without GetVolumeInformationW
    // discrimination (deferred to PR9), we treat all Windows FS as exotic
    // so the Layer 1 watcher (PR8) falls back to PollWatcher 2-5s. This
    // trades local-NTFS latency for "never silently stale" semantics.
    let tmp = TempDir::new().unwrap();
    let checker = StalenessChecker::new(tmp.path().to_path_buf());
    let zero = [0u8; 32];
    let result = checker.check(&zero).expect("check on local tmp");
    assert!(
        result.degraded,
        "Windows arm must default to degraded=true (forces PollWatcher fallback); \
         got degraded=false on {:?}",
        tmp.path()
    );
}
