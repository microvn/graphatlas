//! S-002 AS-004 — install.sh end-to-end with a local fake release directory.
//!
//! Strategy: serve a fake "release" via the `file://` URL scheme. Bash's
//! `curl` supports `file://` so we don't need a test HTTP server. Build a
//! minimal tarball with a mock graphatlas binary, compute its sha256, point
//! GRAPHATLAS_RELEASE_BASE at the file:// URL, and run install.sh.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn have_curl() -> bool {
    Command::new("curl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn compute_sha256_hex(path: &Path) -> String {
    // Use shell utility so we match the same tool install.sh uses.
    let output = if Command::new("sha256sum").arg(path).output().is_ok() {
        Command::new("sha256sum").arg(path).output().unwrap()
    } else {
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
            .unwrap()
    };
    let s = String::from_utf8_lossy(&output.stdout);
    s.split_whitespace().next().unwrap().to_string()
}

fn detect_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "x86_64") => "linux-x86_64-gnu",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "darwin-arm64", // test host fallback
    }
}

#[test]
fn install_sh_happy_path_downloads_and_installs() {
    if !have_curl() {
        eprintln!("skipping: curl not available");
        return;
    }

    let tmp = TempDir::new().unwrap();
    let release = tmp.path().join("release");
    fs::create_dir_all(&release).unwrap();

    // Create the fake tarball for current host target.
    let target = detect_target();
    let payload_dir = tmp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    let fake_bin = payload_dir.join("graphatlas");
    fs::write(&fake_bin, "#!/bin/sh\necho graphatlas-fake\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let tar_name = format!("graphatlas-{target}.tar.gz");
    let tar_path = release.join(&tar_name);
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&tar_path)
        .arg("-C")
        .arg(&payload_dir)
        .arg("graphatlas")
        .status()
        .unwrap();
    assert!(status.success(), "tar failed");

    // Write sha256 sibling.
    let sha_hex = compute_sha256_hex(&tar_path);
    fs::write(
        format!("{}.sha256", tar_path.display()),
        format!("{sha_hex}  {tar_name}\n"),
    )
    .unwrap();

    let bin_dir = tmp.path().join("bin");

    let status = Command::new("bash")
        .arg(std::env::current_dir().unwrap().join("install.sh"))
        .env(
            "GRAPHATLAS_RELEASE_BASE",
            format!("file://{}", release.display()),
        )
        .env("GRAPHATLAS_BIN_DIR", &bin_dir)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .status()
        .unwrap();
    assert!(status.success(), "install.sh exited non-zero");

    let installed = bin_dir.join("graphatlas");
    assert!(installed.exists(), "binary not installed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&installed).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }
    let content = fs::read_to_string(&installed).unwrap();
    assert!(content.contains("graphatlas-fake"));
}

#[test]
fn install_sh_fails_on_sha256_mismatch() {
    if !have_curl() {
        eprintln!("skipping: curl not available");
        return;
    }

    let tmp = TempDir::new().unwrap();
    let release = tmp.path().join("release");
    fs::create_dir_all(&release).unwrap();
    let target = detect_target();
    let tar_name = format!("graphatlas-{target}.tar.gz");

    // Build real tarball but write WRONG sha256 so verification fails.
    let payload_dir = tmp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("graphatlas"), "binary-content").unwrap();
    let tar_path = release.join(&tar_name);
    Command::new("tar")
        .args(["-czf"])
        .arg(&tar_path)
        .arg("-C")
        .arg(&payload_dir)
        .arg("graphatlas")
        .status()
        .unwrap();
    fs::write(
        format!("{}.sha256", tar_path.display()),
        "0000000000000000000000000000000000000000000000000000000000000000  forged\n",
    )
    .unwrap();

    let bin_dir = tmp.path().join("bin");
    let output = Command::new("bash")
        .arg(std::env::current_dir().unwrap().join("install.sh"))
        .env(
            "GRAPHATLAS_RELEASE_BASE",
            format!("file://{}", release.display()),
        )
        .env("GRAPHATLAS_BIN_DIR", &bin_dir)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "install.sh should fail on sha256 mismatch"
    );
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.to_lowercase().contains("sha256") && err.contains("mismatch"),
        "stderr missing sha256 mismatch msg: {err}"
    );
    // Binary must NOT have been installed.
    assert!(!bin_dir.join("graphatlas").exists());
}

#[test]
fn install_sh_skip_sha_env_bypasses_check() {
    if !have_curl() {
        eprintln!("skipping: curl not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let release = tmp.path().join("release");
    fs::create_dir_all(&release).unwrap();
    let target = detect_target();
    let payload_dir = tmp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("graphatlas"), "bin\n").unwrap();
    let tar_path = release.join(format!("graphatlas-{target}.tar.gz"));
    Command::new("tar")
        .args(["-czf"])
        .arg(&tar_path)
        .arg("-C")
        .arg(&payload_dir)
        .arg("graphatlas")
        .status()
        .unwrap();
    // No sha256 file — skip check should not try to fetch it.

    let bin_dir = tmp.path().join("bin");
    let status = Command::new("bash")
        .arg(std::env::current_dir().unwrap().join("install.sh"))
        .env(
            "GRAPHATLAS_RELEASE_BASE",
            format!("file://{}", release.display()),
        )
        .env("GRAPHATLAS_BIN_DIR", &bin_dir)
        .env("GRAPHATLAS_SKIP_SHA256", "1")
        .status()
        .unwrap();
    assert!(status.success());
    assert!(bin_dir.join("graphatlas").exists());
}

#[test]
fn install_sh_prints_path_hint_when_bin_dir_not_in_path() {
    if !have_curl() {
        eprintln!("skipping: curl not available");
        return;
    }
    let tmp = TempDir::new().unwrap();
    let release = tmp.path().join("release");
    fs::create_dir_all(&release).unwrap();
    let target = detect_target();
    let payload_dir = tmp.path().join("payload");
    fs::create_dir_all(&payload_dir).unwrap();
    fs::write(payload_dir.join("graphatlas"), "bin\n").unwrap();
    let tar_path = release.join(format!("graphatlas-{target}.tar.gz"));
    Command::new("tar")
        .args(["-czf"])
        .arg(&tar_path)
        .arg("-C")
        .arg(&payload_dir)
        .arg("graphatlas")
        .status()
        .unwrap();
    let bin_dir = tmp.path().join("bin-hint-test");

    let output = Command::new("bash")
        .arg(std::env::current_dir().unwrap().join("install.sh"))
        .env(
            "GRAPHATLAS_RELEASE_BASE",
            format!("file://{}", release.display()),
        )
        .env("GRAPHATLAS_BIN_DIR", &bin_dir)
        .env("GRAPHATLAS_SKIP_SHA256", "1")
        .env("PATH", "/usr/bin:/bin") // definitely doesn't include bin_dir
        .output()
        .unwrap();
    assert!(output.status.success());
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        out.contains("not in your PATH"),
        "expected PATH hint; got: {out}"
    );
}
