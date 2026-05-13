//! `graphatlas doctor` — diagnose install + cache health (AS-006).
//!
//! Five checks, each producing `Ok` or `Fail` with a remediation hint:
//!   1. Binary in PATH
//!   2. MCP config exists + valid JSON
//!   3. `graphatlas` entry present in MCP config
//!   4. Cache dir writable (0700 on Unix)
//!   5. Fixture spike repo accessible (dev-only; ✓ when $GRAPHATLAS_FIXTURE unset)

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct DoctorOptions {
    pub binary_path: Option<PathBuf>,
    pub mcp_config_path: Option<PathBuf>,
    pub cache_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Fail,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}

impl DoctorReport {
    pub fn all_ok(&self) -> bool {
        self.checks.iter().all(|c| c.status == CheckStatus::Ok)
    }

    pub fn exit_code(&self) -> i32 {
        if self.all_ok() {
            0
        } else {
            1
        }
    }
}

pub fn run_doctor(opts: &DoctorOptions) -> DoctorReport {
    DoctorReport {
        checks: vec![
            check_binary_in_path(opts),
            check_mcp_config_valid(opts),
            check_graphatlas_entry(opts),
            check_cache_dir_writable(opts),
            check_fixture_repo(opts),
        ],
    }
}

fn check_binary_in_path(opts: &DoctorOptions) -> Check {
    let name = "Binary in PATH".to_string();
    let path = match &opts.binary_path {
        Some(p) => p.clone(),
        None => match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                return Check {
                    name,
                    status: CheckStatus::Fail,
                    message: format!("cannot resolve current exe: {e}"),
                    remediation: Some(
                        "reinstall graphatlas via install.sh from GitHub Releases".into(),
                    ),
                };
            }
        },
    };
    if !path.exists() {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: format!("binary {} does not exist", path.display()),
            remediation: Some("reinstall graphatlas via install.sh from GitHub Releases".into()),
        };
    }
    Check {
        name,
        status: CheckStatus::Ok,
        message: format!("found at {}", path.display()),
        remediation: None,
    }
}

fn check_mcp_config_valid(opts: &DoctorOptions) -> Check {
    let name = "MCP config valid JSON".to_string();
    let path = match &opts.mcp_config_path {
        Some(p) => p.clone(),
        None => PathBuf::from("<unresolved>"),
    };
    if !path.exists() {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: format!("{} not found", path.display()),
            remediation: Some("run `graphatlas install --client claude` (or cursor/cline)".into()),
        };
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            return Check {
                name,
                status: CheckStatus::Fail,
                message: format!("read {} failed: {e}", path.display()),
                remediation: Some("check file permissions".into()),
            };
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(_) => Check {
            name,
            status: CheckStatus::Ok,
            message: format!("{} parses as JSON", path.display()),
            remediation: None,
        },
        Err(e) => Check {
            name,
            status: CheckStatus::Fail,
            message: format!("{} is corrupt JSON: {e}", path.display()),
            remediation: Some(format!(
                "restore from {}.bak or run `graphatlas install --client <name>`",
                path.display()
            )),
        },
    }
}

fn check_graphatlas_entry(opts: &DoctorOptions) -> Check {
    let name = "graphatlas entry in MCP config".to_string();
    let Some(path) = opts.mcp_config_path.as_ref() else {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: "no MCP config path configured".into(),
            remediation: Some("run `graphatlas install --client claude`".into()),
        };
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: format!("{} unreadable", path.display()),
            remediation: Some("run `graphatlas install --client claude`".into()),
        };
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: "config is not valid JSON (see previous check)".into(),
            remediation: Some("fix JSON then re-run doctor".into()),
        };
    };
    if doc
        .get("mcpServers")
        .and_then(|s| s.get("graphatlas"))
        .is_some()
    {
        Check {
            name,
            status: CheckStatus::Ok,
            message: format!("graphatlas registered in {}", path.display()),
            remediation: None,
        }
    } else {
        Check {
            name,
            status: CheckStatus::Fail,
            message: "no mcpServers.graphatlas key in config".into(),
            remediation: Some("run `graphatlas install --client claude`".into()),
        }
    }
}

fn check_cache_dir_writable(opts: &DoctorOptions) -> Check {
    let name = "Cache dir writable".to_string();
    let Some(dir) = opts.cache_root.as_ref() else {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: "cache root not set".into(),
            remediation: Some("set $GRAPHATLAS_CACHE_DIR or ensure ~/.graphatlas exists".into()),
        };
    };
    let meta = match std::fs::metadata(dir) {
        Ok(m) => m,
        Err(e) => {
            return Check {
                name,
                status: CheckStatus::Fail,
                message: format!("{} not accessible: {e}", dir.display()),
                remediation: Some(format!(
                    "mkdir -p {} && chmod 0700 {}",
                    dir.display(),
                    dir.display()
                )),
            };
        }
    };
    if !meta.is_dir() {
        return Check {
            name,
            status: CheckStatus::Fail,
            message: format!("{} exists but is not a directory", dir.display()),
            remediation: Some(format!("remove {} and re-run doctor", dir.display())),
        };
    }
    let probe = dir.join(".doctor-probe");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Check {
                name,
                status: CheckStatus::Ok,
                message: format!("{} writable", dir.display()),
                remediation: None,
            }
        }
        Err(e) => Check {
            name,
            status: CheckStatus::Fail,
            message: format!("cannot write in {}: {e}", dir.display()),
            remediation: Some(format!("chmod u+w {}", dir.display())),
        },
    }
}

fn check_fixture_repo(_opts: &DoctorOptions) -> Check {
    let name = "Fixture spike repo".to_string();
    match std::env::var("GRAPHATLAS_FIXTURE") {
        Ok(p) => {
            let path = Path::new(&p);
            if path.exists() {
                Check {
                    name,
                    status: CheckStatus::Ok,
                    message: format!("fixture at {} accessible", path.display()),
                    remediation: None,
                }
            } else {
                Check {
                    name,
                    status: CheckStatus::Fail,
                    message: format!("$GRAPHATLAS_FIXTURE={} not found", path.display()),
                    remediation: Some(
                        "unset $GRAPHATLAS_FIXTURE or point it at a real repo".into(),
                    ),
                }
            }
        }
        Err(_) => Check {
            name,
            status: CheckStatus::Ok,
            message: "$GRAPHATLAS_FIXTURE unset — dev-only check skipped".into(),
            remediation: None,
        },
    }
}
