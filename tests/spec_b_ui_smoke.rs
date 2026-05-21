//! Spec B end-to-end smoke — `ga ui --ui-dir prototype-react` serves
//! the built React app + JS + CSS via ga-server static fallback.
//!
//! Skipped if `prototype-react/` doesn't exist (CI without `bun build`).
//! Run locally via:
//!   cd ui && bun build index.html --outdir ../prototype-react
//!   cargo test --test spec_b_ui_smoke -- --nocapture

use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn spec_b_ui_bundle_served_via_ga_server() {
    let bundle_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prototype-react");
    if !bundle_dir.is_dir() {
        eprintln!(
            "SKIP: {} missing — run `cd ui && bun build index.html --outdir ../prototype-react`",
            bundle_dir.display()
        );
        return;
    }

    let cache = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    let port = 15500 + (std::process::id() % 800) as u16;
    let bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_graphatlas"));
    let mut child = Command::new(&bin)
        .arg("ui")
        .arg("--port")
        .arg(port.to_string())
        .arg("--frontend-port")
        .arg((port + 1).to_string())
        .arg("--ui-dir")
        .arg(&bundle_dir)
        .arg("--no-open")
        .env("GRAPHATLAS_CACHE_DIR", cache.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn graphatlas ui");

    // Wait for port.
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready = false;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        panic!("ga-server port {} never opened", port);
    }

    let index = http_get(&format!("http://127.0.0.1:{}/", port)).expect("/");
    assert!(
        index.contains("<div id=\"root\">"),
        "React mount missing from index.html: {}",
        index
    );
    // Extract JS bundle filename + fetch it.
    let bundle_ref = index
        .split_terminator(|c| c == '"' || c == '\'')
        .find(|s| s.starts_with("./index-") && s.ends_with(".js"))
        .or_else(|| {
            index
                .split_terminator(|c| c == '"' || c == '\'')
                .find(|s| s.starts_with("/index-") && s.ends_with(".js"))
        })
        .expect("JS bundle reference in index.html");
    let bundle_path = bundle_ref.trim_start_matches('.');
    let bundle_url = format!("http://127.0.0.1:{}{}", port, bundle_path);
    let bundle_status = http_status(&bundle_url);
    assert_eq!(bundle_status, 200, "bundle not served: {bundle_url}");

    // /api/health still works alongside static fallback.
    let health = http_get(&format!("http://127.0.0.1:{}/api/health", port)).expect("health");
    assert!(health.contains("\"status\":\"ok\""), "health: {health}");

    // SIGINT clean exit.
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg("-INT")
            .arg(child.id().to_string())
            .status();
    }
    let _ = wait_with_timeout(&mut child, Duration::from_secs(10));
}

fn http_get(url: &str) -> Result<String, String> {
    let url = url
        .strip_prefix("http://")
        .ok_or_else(|| "http only".to_string())?;
    let (host, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    use std::io::{Read, Write};
    let mut s = TcpStream::connect(host).map_err(|e| e.to_string())?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut resp = String::new();
    s.read_to_string(&mut resp).map_err(|e| e.to_string())?;
    let split = resp.find("\r\n\r\n").unwrap_or(0);
    Ok(resp.split_at(split + 4).1.to_string())
}

fn http_status(url: &str) -> u16 {
    let url = match url.strip_prefix("http://") {
        Some(u) => u,
        None => return 0,
    };
    let (host, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    use std::io::{Read, Write};
    let Ok(mut s) = TcpStream::connect(host) else {
        return 0;
    };
    let req = format!("HEAD {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return 0;
    }
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]);
    head.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

fn wait_with_timeout(child: &mut std::process::Child, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return false,
        }
    }
}
