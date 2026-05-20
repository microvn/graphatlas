//! ServerConfig — runtime parameters resolved by the CLI layer
//! (Spec A S-001 AS-001/AS-005/AS-009 + Spec D S-001 token bootstrap).

use std::path::PathBuf;

/// Immutable per-process config. Cloned into AppState via Arc.
#[derive(Debug)]
pub struct ServerConfig {
    /// Loopback bind addr — validated by `validate_bind_addr` (AS-002).
    pub bind: std::net::SocketAddr,
    /// Cache root (default `~/.graphatlas`). S-002 endpoints scan this.
    pub cache_root: PathBuf,
    /// Per-session token (hex). Required on `X-GA-Token` header for any
    /// non-health endpoint (AS-005). 32-byte / 64-hex-char minimum.
    pub token: String,
    /// Allowed Origin values (AS-003). Frontend host:port the user
    /// configured `ga ui` to launch. Typically `http://localhost:4318`
    /// + `http://127.0.0.1:4318`.
    pub allowed_origins: Vec<String>,
    /// Allowed Host header values (AS-004). Should include
    /// `127.0.0.1:<backend_port>` + `localhost:<backend_port>`.
    pub allowed_hosts: Vec<String>,
    /// The frontend URL ga-server reports through `/api/config` so the
    /// browser knows where Bun.serve is.
    pub frontend_origin: String,
}

impl ServerConfig {
    /// Build the canonical Origin allowlist for a frontend port —
    /// `http://localhost:N` + `http://127.0.0.1:N`. Order matters only
    /// for readability; the middleware does a flat equality check.
    pub fn origins_for_port(port: u16) -> Vec<String> {
        vec![
            format!("http://localhost:{}", port),
            format!("http://127.0.0.1:{}", port),
        ]
    }

    /// Allowlist for D-S001 single-process orchestration where ga-server
    /// serves both `/api` AND the static UI on the SAME port. Browser
    /// Origin will be the backend port, not the (legacy) frontend port —
    /// so we need both.
    pub fn origins_for_single_process(backend_port: u16, frontend_port: u16) -> Vec<String> {
        let mut v = Self::origins_for_port(backend_port);
        if backend_port != frontend_port {
            v.extend(Self::origins_for_port(frontend_port));
        }
        v
    }

    /// Build the canonical Host allowlist for the backend port.
    pub fn hosts_for_port(port: u16) -> Vec<String> {
        vec![
            format!("127.0.0.1:{}", port),
            format!("localhost:{}", port),
        ]
    }
}
