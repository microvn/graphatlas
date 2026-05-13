//! AS-031 extra secret rules not covered in tests/walk.rs:
//!   - `.ssh/` as a directory segment excludes the whole subtree
//!   - Unix: file with mode 0600 AND no source extension → treated as a
//!     probable secret and skipped

use ga_parser::walk::walk_repo;
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
fn ssh_dir_segment_excludes_whole_subtree() {
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    // Files anywhere inside a .ssh segment must be skipped.
    write(&tmp.path().join(".ssh/config"), "Host x");
    write(&tmp.path().join(".ssh/authorized_keys"), "ssh-rsa ...");
    write(&tmp.path().join("home/.ssh/script.py"), "# secret-adjacent");
    let report = walk_repo(tmp.path()).unwrap();
    let names: Vec<String> = report
        .entries
        .iter()
        .map(|e| e.rel_path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "only app.py should remain: {names:?}");
    assert!(names[0].ends_with("app.py"));
}

#[cfg(unix)]
#[test]
fn file_0600_without_source_ext_is_treated_as_secret() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");

    // Secret-shaped by mode: 0600 with no recognized source extension.
    let secret_path = tmp.path().join("api-token");
    write(&secret_path, "abc123");
    fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o600)).unwrap();

    // 0600 file that DOES have a source extension is fine (users may lock
    // down their own source files).
    let protected_src = tmp.path().join("protected.py");
    write(&protected_src, "print(1)");
    fs::set_permissions(&protected_src, fs::Permissions::from_mode(0o600)).unwrap();

    let report = walk_repo(tmp.path()).unwrap();
    let names: Vec<String> = report
        .entries
        .iter()
        .map(|e| e.rel_path.to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|p| p.ends_with("app.py")), "{names:?}");
    assert!(
        names.iter().any(|p| p.ends_with("protected.py")),
        "0600 + recognized ext is OK: {names:?}"
    );
    assert!(
        !names.iter().any(|p| p.contains("api-token")),
        "api-token (0600, no source ext) must be skipped: {names:?}"
    );
    // AS-031 requires the file be TRACKED in skipped_secrets (so doctor can
    // surface it), not just silently ignored.
    let tracked: Vec<String> = report
        .skipped_secrets
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    assert!(
        tracked.iter().any(|p| p.contains("api-token")),
        "api-token should be in skipped_secrets: {tracked:?}"
    );
}

#[cfg(unix)]
#[test]
fn file_0644_without_source_ext_is_not_auto_secret() {
    // Control: random non-source files at normal mode should be filtered
    // OUT simply because we only index recognized source extensions — NOT
    // because of the 0600 secret rule. This test guards against false-
    // positive broadening.
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    write(&tmp.path().join("app.py"), "");
    write(&tmp.path().join("notes.txt"), "todo");
    fs::set_permissions(
        tmp.path().join("notes.txt"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let report = walk_repo(tmp.path()).unwrap();
    // notes.txt skipped for extension reasons, not secret reasons.
    assert!(
        !report
            .skipped_secrets
            .iter()
            .any(|p| p.to_string_lossy().contains("notes.txt")),
        "0644 non-source file should NOT be classified as secret: {:?}",
        report.skipped_secrets
    );
}
