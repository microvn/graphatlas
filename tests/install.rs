//! S-002 AS-005 — `graphatlas install --client <name>` MCP config writer.

use graphatlas::install::{write_mcp_config, Client, InstallOutcome};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn read_json(p: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

#[test]
fn writes_fresh_mcp_config_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    let outcome = write_mcp_config(
        Client::Claude,
        Some(&cfg),
        Path::new("/usr/local/bin/graphatlas"),
    )
    .unwrap();
    match outcome {
        InstallOutcome::Created { config_path, .. } => {
            assert_eq!(config_path, cfg);
        }
        other => panic!("expected Created, got {other:?}"),
    }
    let v = read_json(&cfg);
    assert_eq!(
        v["mcpServers"]["graphatlas"]["command"],
        "/usr/local/bin/graphatlas"
    );
    let args = v["mcpServers"]["graphatlas"]["args"].as_array().unwrap();
    assert_eq!(args, &[serde_json::Value::String("mcp".to_string())]);
}

#[test]
fn preserves_existing_mcp_servers_on_merge() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(
        &cfg,
        r#"{
  "mcpServers": {
    "otherSrv": { "command": "node", "args": ["server.js"] },
    "another":  { "command": "python3", "args": ["-m", "srv"] }
  },
  "unrelatedKey": 42
}"#,
    )
    .unwrap();

    let outcome = write_mcp_config(
        Client::Claude,
        Some(&cfg),
        Path::new("/opt/graphatlas/bin/graphatlas"),
    )
    .unwrap();
    assert!(
        matches!(outcome, InstallOutcome::Updated { .. }),
        "outcome: {outcome:?}"
    );

    let v = read_json(&cfg);
    // New entry added.
    assert_eq!(
        v["mcpServers"]["graphatlas"]["command"],
        "/opt/graphatlas/bin/graphatlas"
    );
    // Existing entries survived.
    assert_eq!(v["mcpServers"]["otherSrv"]["command"], "node");
    assert_eq!(v["mcpServers"]["another"]["command"], "python3");
    // Unrelated top-level key preserved.
    assert_eq!(v["unrelatedKey"], 42);
}

#[test]
fn backup_file_created_when_config_existed() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    let original = r#"{"mcpServers":{}}"#;
    fs::write(&cfg, original).unwrap();

    write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();

    let backup = tmp.path().join("mcp.json.bak");
    assert!(backup.exists(), "expected backup file");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
}

#[test]
fn no_backup_when_config_did_not_exist() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();
    assert!(!tmp.path().join("mcp.json.bak").exists());
}

#[test]
fn idempotent_rewrite_is_safe() {
    // Running install twice must produce identical final state.
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();
    let first = fs::read_to_string(&cfg).unwrap();
    write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();
    let second = fs::read_to_string(&cfg).unwrap();
    assert_eq!(first, second, "second install must be identical");
}

#[test]
fn corrupt_json_returns_error_without_destroying_file() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    fs::write(&cfg, "{not json at all").unwrap();

    let err = write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas"))
        .expect_err("corrupt JSON should error");
    assert!(
        format!("{err}").to_lowercase().contains("corrupt")
            || format!("{err}").to_lowercase().contains("config")
            || format!("{err}").to_lowercase().contains("json")
    );
    // Original file bytes preserved.
    let bytes = fs::read_to_string(&cfg).unwrap();
    assert_eq!(bytes, "{not json at all");
}

#[test]
fn all_three_clients_are_supported() {
    for client in [Client::Claude, Client::Cursor, Client::Cline] {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("mcp.json");
        let outcome = write_mcp_config(client, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();
        assert!(matches!(outcome, InstallOutcome::Created { .. }));
        let v = read_json(&cfg);
        assert_eq!(v["mcpServers"]["graphatlas"]["command"], "/bin/graphatlas");
    }
}

#[test]
fn client_parses_from_string() {
    use std::str::FromStr;
    assert_eq!(Client::from_str("claude").unwrap(), Client::Claude);
    assert_eq!(Client::from_str("cursor").unwrap(), Client::Cursor);
    assert_eq!(Client::from_str("cline").unwrap(), Client::Cline);
    assert_eq!(Client::from_str("CLAUDE").unwrap(), Client::Claude);
    assert!(Client::from_str("unknown").is_err());
}

#[test]
fn args_field_is_written_atomically_via_tmp_file() {
    // After write, no .tmp leftover in parent dir.
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("mcp.json");
    write_mcp_config(Client::Claude, Some(&cfg), Path::new("/bin/graphatlas")).unwrap();
    let leftover: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftover.is_empty(), "leftover tmp: {leftover:?}");
}
