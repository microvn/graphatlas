//! S-002 AS-006 — `graphatlas doctor` health check.
//!
//! 5 checks per spec:
//!   1. binary in PATH
//!   2. MCP config valid JSON
//!   3. `graphatlas` entry present in config
//!   4. cache dir writable
//!   5. fixture spike repo accessible (dev-only, always ✓ when $GRAPHATLAS_FIXTURE unset)

use graphatlas::doctor::{run_doctor, CheckStatus, DoctorOptions, DoctorReport};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn opts(mcp_config: &Path, cache_root: &Path) -> DoctorOptions {
    DoctorOptions {
        binary_path: Some(std::path::PathBuf::from(env!("CARGO_BIN_EXE_graphatlas"))),
        mcp_config_path: Some(mcp_config.to_path_buf()),
        cache_root: Some(cache_root.to_path_buf()),
    }
}

#[test]
fn all_green_on_healthy_setup() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(
        &cfg,
        r#"{"mcpServers":{"graphatlas":{"command":"/bin/graphatlas","args":["mcp"]}}}"#,
    )
    .unwrap();
    let cache = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let report: DoctorReport = run_doctor(&opts(&cfg, &cache));
    for check in &report.checks {
        assert_eq!(
            check.status,
            CheckStatus::Ok,
            "{}: {}",
            check.name,
            check.message
        );
    }
    assert!(report.all_ok());
}

#[test]
fn mcp_config_missing_file_fails_with_remediation() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does-not-exist.json");
    let cache = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let report = run_doctor(&opts(&missing, &cache));
    let cfg_check = report
        .checks
        .iter()
        .find(|c| c.name.contains("MCP config"))
        .expect("mcp config check missing");
    assert_eq!(cfg_check.status, CheckStatus::Fail);
    assert!(cfg_check.remediation.as_ref().unwrap().contains("install"));
    assert!(!report.all_ok());
}

#[test]
fn corrupt_mcp_config_fails_check() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(&cfg, "{not json").unwrap();
    let cache = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let report = run_doctor(&opts(&cfg, &cache));
    let cfg_check = report
        .checks
        .iter()
        .find(|c| c.name.contains("MCP config"))
        .unwrap();
    assert_eq!(cfg_check.status, CheckStatus::Fail);
    assert!(
        cfg_check.message.to_lowercase().contains("json")
            || cfg_check.message.to_lowercase().contains("corrupt")
    );
}

#[test]
fn missing_graphatlas_entry_fails_specific_check() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(&cfg, r#"{"mcpServers":{"someOther":{"command":"node"}}}"#).unwrap();
    let cache = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let report = run_doctor(&opts(&cfg, &cache));
    let entry_check = report
        .checks
        .iter()
        .find(|c| c.name.contains("entry"))
        .unwrap();
    assert_eq!(entry_check.status, CheckStatus::Fail);
    assert!(entry_check
        .remediation
        .as_ref()
        .unwrap()
        .contains("install"));
}

#[test]
fn cache_dir_not_writable_fails() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(
        &cfg,
        r#"{"mcpServers":{"graphatlas":{"command":"/bin/graphatlas","args":["mcp"]}}}"#,
    )
    .unwrap();
    // Point cache_root at a FILE (not a dir) → writability test will fail.
    let not_a_dir = tmp.path().join("iam-a-file");
    fs::write(&not_a_dir, "").unwrap();

    let report = run_doctor(&opts(&cfg, &not_a_dir));
    let cache_check = report
        .checks
        .iter()
        .find(|c| c.name.contains("Cache dir"))
        .unwrap();
    assert_eq!(cache_check.status, CheckStatus::Fail);
}

#[test]
fn report_has_all_five_checks() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(&cfg, r#"{"mcpServers":{}}"#).unwrap();
    let report = run_doctor(&opts(&cfg, tmp.path()));
    assert_eq!(report.checks.len(), 5, "{:#?}", report.checks);
    let names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.iter().any(|n| n.contains("Binary")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("MCP config")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("entry")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("Cache dir")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("Fixture")), "{names:?}");
}

#[test]
fn exit_code_pass() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(
        &cfg,
        r#"{"mcpServers":{"graphatlas":{"command":"/bin/graphatlas","args":["mcp"]}}}"#,
    )
    .unwrap();
    let cache = tmp.path().join(".graphatlas");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let report = run_doctor(&opts(&cfg, &cache));
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn exit_code_fail() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("nope.json");
    let report = run_doctor(&opts(&missing, tmp.path()));
    assert_eq!(report.exit_code(), 1);
}
