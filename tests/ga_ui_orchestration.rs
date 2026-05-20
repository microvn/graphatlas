//! D-S001 follow-up — end-to-end orchestration smoke for `ga ui`.
//!
//! Spawns the real `graphatlas` binary in `ui` mode against a tempdir
//! cache + tempdir frontend, polls `/api/health` until the port comes
//! up, asserts the static `index.html` lands at `/`, then sends SIGINT
//! and verifies clean exit. Real subprocess + real backend + real
//! signal handling.

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn ga_ui_spawns_backend_serves_health_and_static() {
    // 1. Tempdir cache (mode 0700 per Foundation H-2).
    let cache = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    // 2. Tempdir UI with index.html.
    let ui = tempfile::tempdir().unwrap();
    let marker = "<h1>orchestration smoke ✓</h1>";
    std::fs::write(ui.path().join("index.html"), marker).unwrap();

    // 3. Free-ish port. Just pick high-range; collisions on a busy
    // dev box can be retried with --port.
    let port = 14500 + (std::process::id() % 800) as u16;

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_graphatlas"));

    let mut child = Command::new(&bin)
        .arg("ui")
        .arg("--port").arg(port.to_string())
        .arg("--frontend-port").arg((port + 1).to_string())
        .arg("--ui-dir").arg(ui.path())
        .arg("--no-open")
        .env("GRAPHATLAS_CACHE_DIR", cache.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn graphatlas ui");

    // 4. Wait for port to come up (cmd_ui's own health probe is doing
    // the same thing inside — we just observe from outside).
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
        panic!("ga-server port {} never opened within 20s", port);
    }

    // 5. /api/health should return JSON 200 (open endpoint, no token).
    let health_url = format!("http://127.0.0.1:{}/api/health", port);
    let body = http_get(&health_url).expect("GET /api/health");
    assert!(
        body.contains("\"status\":\"ok\""),
        "health body unexpected: {body}"
    );

    // 6. `/` should serve the static index.html via --ui-dir.
    let root_url = format!("http://127.0.0.1:{}/", port);
    let body = http_get(&root_url).expect("GET /");
    assert!(
        body.contains("orchestration smoke"),
        "static index.html not served at /: {body}"
    );

    // 7. SIGINT to the orchestrator → both processes exit cleanly.
    #[cfg(unix)]
    {
        // signal-hook-less SIGINT: shell out to `kill -INT <pid>`.
        let _ = Command::new("kill")
            .arg("-INT")
            .arg(child.id().to_string())
            .status();
    }
    // Wait for exit; bounded so the test doesn't hang.
    let exited = wait_with_timeout(&mut child, Duration::from_secs(10));
    assert!(exited, "graphatlas ui didn't exit within 10s after SIGINT");
}

/// Minimal blocking HTTP/1.1 GET using just `std::net`. Avoids pulling
/// in an HTTP client crate just for tests.
fn http_get(url: &str) -> Result<String, String> {
    let url = url
        .strip_prefix("http://")
        .ok_or_else(|| "only http:// supported".to_string())?;
    let (host, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    use std::io::{Read, Write};
    let mut stream = TcpStream::connect(host).map_err(|e| format!("connect: {e}"))?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n",
        path = path,
        host = host
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| format!("read: {e}"))?;
    Ok(resp)
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
