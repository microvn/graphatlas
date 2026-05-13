//! v1.5 Tier B — real subprocess smoke test for the reindex lifecycle.
//!
//! Boots the `graphatlas` release binary as a child process, drives the
//! rmcp newline-delimited JSON-RPC handshake over its stdin/stdout, and
//! exercises the critical lifecycle paths end-to-end:
//!
//!  1. initialize + tools/list (handshake works on the real subprocess)
//!  2. baseline tools/call ga_callers
//!  3. edit a tracked file → next call hits the PR5 staleness gate
//!     (-32010 STALE_INDEX)
//!  4. tools/call ga_reindex full → response includes
//!     `graph_generation_after`
//!  5. post-reindex tools/call ga_callers returns fresh data
//!  6. external `git commit` → L1 watcher dispatches ga_reindex,
//!     stderr line `L1 watcher: reindex complete` confirms autofire
//!
//! Why a subprocess test on top of `stdio_integration.rs`: that file
//! drives `serve_with_store` IN-PROCESS via tokio::io::duplex. This one
//! spawns the actual binary so we catch issues that only show up
//! after `cargo build --release` (linker flags, tracing init, the
//! cmd_mcp boot path, watcher thread). Closes the last gap between
//! in-process E2E and reality.
//!
//! Ignored under default `cargo test` because it costs a release build.
//! Run via:
//!
//! ```bash
//! cargo test --release --test smoke_reindex_subprocess -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_graphatlas")
}

struct McpChild {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    log_path: PathBuf,
    child: tokio::process::Child,
    next_id: u64,
}

impl McpChild {
    async fn spawn(repo: &PathBuf, cache: &PathBuf, log: PathBuf) -> Self {
        let log_file = std::fs::File::create(&log).expect("create mcp log");
        let mut child = Command::new(bin())
            .arg("mcp")
            .current_dir(repo)
            .env("RUST_LOG", "info")
            .env("GRAPHATLAS_CACHE_DIR", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log_file)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn graphatlas mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            stdin,
            stdout,
            log_path: log,
            child,
            next_id: 0,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    async fn send(&mut self, msg: &serde_json::Value) {
        let mut s = msg.to_string();
        s.push('\n');
        self.stdin
            .write_all(s.as_bytes())
            .await
            .expect("write stdin");
        self.stdin.flush().await.expect("flush stdin");
    }

    async fn recv(&mut self, deadline: Duration) -> serde_json::Value {
        let mut line = String::new();
        let read = timeout(deadline, self.stdout.read_line(&mut line)).await;
        match read {
            Ok(Ok(0)) => {
                let log = std::fs::read_to_string(&self.log_path).unwrap_or_default();
                panic!("MCP closed stdout unexpectedly\n--- mcp stderr ---\n{log}");
            }
            Ok(Ok(_)) => serde_json::from_str(line.trim()).unwrap_or_else(|e| {
                let log = std::fs::read_to_string(&self.log_path).unwrap_or_default();
                panic!("invalid JSON on stdout: {e}\nline: {line:?}\n--- mcp stderr ---\n{log}");
            }),
            Ok(Err(e)) => panic!("stdout read error: {e}"),
            Err(_) => {
                let log = std::fs::read_to_string(&self.log_path).unwrap_or_default();
                panic!(
                    "no response within {:?}\n--- mcp stderr ---\n{log}",
                    deadline
                );
            }
        }
    }

    async fn call_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
        wait: Duration,
    ) -> serde_json::Value {
        let id = self.id();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        self.send(&req).await;
        self.recv(wait).await
    }
}

fn seed_repo(repo: &PathBuf) {
    use std::process::Command as StdCommand;
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src").join("a.py"),
        "def foo():\n    pass\n\ndef caller():\n    foo()\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("src").join("b.py"),
        "from .a import foo\n\ndef runner():\n    foo()\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "smoke@example.com"],
        vec!["config", "user.name", "smoke"],
        vec!["add", "."],
        vec!["commit", "-q", "-m", "init"],
    ] {
        StdCommand::new("git")
            .args(&args)
            .current_dir(repo)
            .status()
            .unwrap();
    }
}

macro_rules! step {
    ($($arg:tt)*) => {
        eprintln!("  ✓ {}", format!($($arg)*));
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "release subprocess + git ops — run with `cargo test --release -- --ignored --nocapture`"]
async fn smoke_reindex_full_lifecycle() {
    eprintln!();
    eprintln!("smoke_reindex_full_lifecycle — 15-step v1.5 lifecycle gate");
    eprintln!();
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    step!("1/15  TempDir + cache root");
    let cache = tmp.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Cache root must be 0700 — graphatlas refuses to use a world-
        // readable cache dir as a defense against credential leaks.
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    step!("2/15  chmod 0700 cache (graphatlas refuses world-readable cache)");
    seed_repo(&repo);
    step!("3/15  git repo seeded with src/a.py + src/b.py + initial commit");

    let log_path = tmp.path().join("mcp.log");
    let mut mcp = McpChild::spawn(&repo, &cache, log_path.clone()).await;
    step!("4/15  spawned `graphatlas mcp` subprocess (RUST_LOG=info)");

    // 1 — initialize handshake.
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": mcp.id(),
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "smoke", "version": "0.0.0" }
        }
    });
    mcp.send(&init).await;
    let init_resp = mcp.recv(Duration::from_secs(30)).await;
    assert_eq!(
        init_resp["result"]["protocolVersion"]
            .as_str()
            .unwrap_or_default(),
        "2025-11-25",
        "initialize must return protocolVersion 2025-11-25; got {init_resp}"
    );
    step!("5/15  initialize handshake → protocolVersion 2025-11-25");

    let initialized_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    mcp.send(&initialized_notif).await;
    step!("6/15  notifications/initialized sent");

    // 2 — tools/list contains ga_reindex.
    let list_id = mcp.id();
    mcp.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": list_id,
        "method": "tools/list"
    }))
    .await;
    let list_resp = mcp.recv(Duration::from_secs(10)).await;
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert!(
        tools.iter().any(|t| t["name"] == "ga_reindex"),
        "tools/list missing ga_reindex; got {list_resp}"
    );
    step!("7/15  tools/list returned {} tools (incl. ga_reindex)", tools.len());

    // 3 — baseline ga_callers.
    let baseline = mcp
        .call_tool(
            "ga_callers",
            serde_json::json!({ "symbol": "foo" }),
            Duration::from_secs(30),
        )
        .await;
    assert!(
        baseline["result"].is_object(),
        "baseline ga_callers must succeed; got {baseline}"
    );
    step!("8/15  baseline ga_callers(symbol=foo) succeeded");

    // 4 — Force a Merkle drift the staleness gate can see. The Merkle
    // root hashes depth ≤2 directory mtimes + .git/HEAD + .git/index.
    // Editing src/a.py only bumps src/'s mtime; coarse FS mtime
    // resolution (1s on some boxes) can mask that. Sleep past the
    // resolution boundary AND add a new subdir so a depth-2 dir
    // appears in the bounded set — guarantees a hash flip.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    std::fs::write(
        repo.join("src").join("a.py"),
        "def foo():\n    return 42\n\ndef caller():\n    foo()\n\ndef new_helper():\n    foo()\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.join("src").join("nested_drift")).unwrap();
    step!("9/15  edited src/a.py + created src/nested_drift/ (Merkle drift)");
    // Outlast the 500ms TTL cache on top of the resolution sleep.
    tokio::time::sleep(Duration::from_millis(700)).await;

    let stale = mcp
        .call_tool(
            "ga_callers",
            serde_json::json!({ "symbol": "foo" }),
            Duration::from_secs(15),
        )
        .await;
    let code = stale["error"]["code"].as_i64();
    assert_eq!(
        code,
        Some(-32010),
        "post-edit ga_callers must hit STALE_INDEX (-32010); got {stale}"
    );
    step!("10/15 staleness gate fired STALE_INDEX (-32010) as expected");

    // 5 — ga_reindex full
    let reindex = mcp
        .call_tool(
            "ga_reindex",
            serde_json::json!({ "mode": "full" }),
            Duration::from_secs(90),
        )
        .await;
    let content_text = reindex["result"]["content"][0]["text"]
        .as_str()
        .expect("content[0].text");
    let payload: serde_json::Value = serde_json::from_str(content_text).expect("nested json");
    assert_eq!(
        payload["reindexed"].as_bool(),
        Some(true),
        "ga_reindex reindexed=true; got {payload}"
    );
    assert!(
        payload["graph_generation_after"].as_u64().is_some(),
        "ga_reindex must report graph_generation_after; got {payload}"
    );
    let gen_after = payload["graph_generation_after"].as_u64().unwrap();
    let files_indexed = payload["files_indexed"].as_u64().unwrap_or(0);
    let took_ms = payload["took_ms"].as_u64().unwrap_or(0);
    step!("11/15 ga_reindex full → reindexed=true, gen_after={gen_after}, files={files_indexed}, took={took_ms}ms");

    // 6 — sleep past the 200ms cooldown then exercise fresh ga_callers.
    tokio::time::sleep(Duration::from_millis(300)).await;
    step!("12/15 sleep 300ms past the 200ms cooldown");
    let fresh = mcp
        .call_tool(
            "ga_callers",
            serde_json::json!({ "symbol": "foo" }),
            Duration::from_secs(15),
        )
        .await;
    assert!(
        fresh["result"].is_object() && fresh["error"].is_null(),
        "post-reindex ga_callers must be fresh, no STALE_INDEX; got {fresh}"
    );
    step!("13/15 post-reindex ga_callers fresh (no STALE_INDEX)");

    // 7 — external git commit must wake the L1 watcher and dispatch
    // a reindex within DEBOUNCE_MS + slack. Grep the stderr log for
    // the spec-literal "L1 watcher: reindex complete" line.
    use std::process::Command as StdCommand;
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .status()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "-q", "-m", "edit"])
        .current_dir(&repo)
        .status()
        .unwrap();
    step!("14/15 external `git commit -m edit` (terminal, not via MCP)");

    let watcher_deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut saw_watcher = false;
    while std::time::Instant::now() < watcher_deadline {
        if let Ok(buf) = std::fs::read_to_string(&log_path) {
            if buf.contains("L1 watcher: reindex complete") {
                saw_watcher = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if !saw_watcher {
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        panic!(
            "L1 watcher never logged 'reindex complete' within 30s after external commit\n\
             --- mcp.log (tail) ---\n{}",
            log.lines()
                .rev()
                .take(50)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    step!("15/15 L1 watcher autofire: stderr saw `reindex complete` within 30s");

    eprintln!();
    eprintln!("✓ all 15 lifecycle steps passed — v1.5 reindex healthy end-to-end");
    eprintln!();

    // Clean shutdown — kill child via Drop (kill_on_drop=true).
    let _ = mcp.child.kill().await;
}
