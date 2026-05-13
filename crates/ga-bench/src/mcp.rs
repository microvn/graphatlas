//! MCP stdio client primitives.
//!
//! Split into two layers:
//!   - [`McpCore`] — pure protocol: build line-delimited JSON-RPC messages,
//!     feed chunks of a stdout stream, extract responses by id. Stateless
//!     w.r.t. I/O → trivially unit-testable.
//!   - [`McpChild`] — production wrapper that spawns a child process, pumps
//!     stdout into an mpsc channel on a reader thread, and drives `McpCore`
//!     with a per-request timeout.
//!
//! Parser rationale: MCP over stdio is line-delimited JSON, but some servers
//! pretty-print responses (multi-line objects with nested braces). The TS
//! adapter layer solves this with a balanced-brace scanner — we port the
//! same approach so the Rust client handles both framings.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

/// MCP spec version per the existing TS adapter layer (`mcp-client.ts`).
/// Servers in this ecosystem (cgc, codebase-memory-mcp, code-review-graph)
/// all accept this version string. GA's own server uses `2025-11-25`, but
/// we're clients talking to THEM here, so we send their dialect.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP RPC error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("MCP timeout after {0:?}")]
    Timeout(Duration),

    #[error("MCP server disconnected")]
    Disconnected,

    #[error("MCP spawn failed: {0}")]
    Spawn(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("MCP malformed response: {0}")]
    Malformed(String),
}

pub struct McpCore {
    buffer: String,
    next_id: u64,
}

impl Default for McpCore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpCore {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            next_id: 0,
        }
    }

    /// Build the `initialize` request + `notifications/initialized` payload
    /// as a single line-delimited byte blob ready for `stdin.write_all`.
    pub fn build_initialize_bytes(&mut self) -> Vec<u8> {
        self.next_id += 1;
        let init_id = self.next_id;
        let init = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "ga-bench", "version": env!("CARGO_PKG_VERSION") },
            }
        });
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        });
        let mut out = String::new();
        out.push_str(&init.to_string());
        out.push('\n');
        out.push_str(&notif.to_string());
        out.push('\n');
        out.into_bytes()
    }

    /// Build a `tools/call` request. Returns the assigned id so the caller
    /// can match the response later via [`Self::try_take_response`].
    pub fn build_tools_call_bytes(&mut self, name: &str, arguments: Value) -> (u64, Vec<u8>) {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let mut line = req.to_string();
        line.push('\n');
        (id, line.into_bytes())
    }

    /// Feed raw stdout bytes (possibly a partial chunk) into the parser.
    pub fn feed(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);
    }

    /// Try to extract the next complete JSON-RPC response matching `id`.
    /// Returns `None` when the buffer holds no complete object yet, or
    /// holds objects for other ids only. Consumed bytes drop from buffer.
    pub fn try_take_response(&mut self, id: u64) -> Option<Value> {
        loop {
            let (parsed, consumed) = next_balanced_object(&self.buffer)?;
            // Consume whatever we parsed, whether it matches or not.
            self.buffer.drain(..consumed);
            if let Some(parsed_id) = parsed.get("id").and_then(|v| v.as_u64()) {
                if parsed_id == id {
                    return Some(parsed);
                }
            }
            // No id or wrong id — keep looping to try the next object.
        }
    }

    /// Classify a JSON-RPC response: `result` branch OK, `error` branch → typed error.
    pub fn result_or_error(resp: &Value) -> Result<Value, McpError> {
        if let Some(err) = resp.get("error") {
            let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-32000);
            let message = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("<no message>")
                .to_string();
            return Err(McpError::Rpc { code, message });
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| McpError::Malformed("response lacks both result and error".into()))
    }
}

/// Balanced-brace scanner — finds the first complete top-level `{...}` in
/// `input` and returns (parsed Value, number of bytes consumed including
/// trailing newline / whitespace up to the next `{`). Returns `None` when
/// input lacks a complete object.
///
/// Honors string-literal quoting + escape sequences so braces inside JSON
/// strings don't throw off the depth counter.
fn next_balanced_object(input: &str) -> Option<(Value, usize)> {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'{' {
        i += 1;
    }
    if i == bytes.len() {
        return None;
    }
    let start = i;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
        } else {
            match c {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let end = i + 1;
                        let slice = &input[start..end];
                        if let Ok(v) = serde_json::from_str::<Value>(slice) {
                            return Some((v, end));
                        } else {
                            return None;
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

// --- Production I/O wrapper ------------------------------------------------

pub struct McpChild {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    core: McpCore,
    _reader: thread::JoinHandle<()>,
}

impl McpChild {
    /// Spawn `cmd`, start a reader thread pumping stdout into a channel, run
    /// the MCP initialize handshake, and return a client ready for
    /// [`Self::tools_call`].
    pub fn spawn(cmd: &[&str], init_timeout: Duration) -> Result<Self, McpError> {
        if cmd.is_empty() {
            return Err(McpError::Spawn("empty command".into()));
        }
        let mut child = Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| McpError::Spawn(format!("{}: {e}", cmd[0])))?;

        let stdin = child
            .stdin
            .take()
            .ok_or(McpError::Malformed("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(McpError::Malformed("no stdout".into()))?;

        let (tx, rx): (Sender<String>, Receiver<String>) = channel();
        let reader = thread::spawn(move || pump_stdout(stdout, tx));

        let mut client = Self {
            child,
            stdin: Some(stdin),
            rx,
            core: McpCore::new(),
            _reader: reader,
        };

        // Perform handshake. Some servers need a pause between init and
        // subsequent requests (per TS adapter note); caller's tools_call
        // adds its own buffer so we skip the delay here.
        let init_bytes = client.core.build_initialize_bytes();
        client.write_stdin(&init_bytes)?;
        // initialize id is 1 (next_id was 0 before the call)
        client.await_response(1, init_timeout)?;
        Ok(client)
    }

    /// Call a tool and block until the response lands or `timeout` elapses.
    pub fn tools_call(
        &mut self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, McpError> {
        let (id, bytes) = self.core.build_tools_call_bytes(name, arguments);
        self.write_stdin(&bytes)?;
        let resp = self.await_response(id, timeout)?;
        McpCore::result_or_error(&resp)
    }

    fn write_stdin(&mut self, bytes: &[u8]) -> Result<(), McpError> {
        let stdin = self.stdin.as_mut().ok_or(McpError::Disconnected)?;
        stdin.write_all(bytes)?;
        stdin.flush()?;
        Ok(())
    }

    fn await_response(&mut self, id: u64, timeout: Duration) -> Result<Value, McpError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(resp) = self.core.try_take_response(id) {
                return Ok(resp);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpError::Timeout(timeout));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(chunk) => self.core.feed(&chunk),
                Err(RecvTimeoutError::Timeout) => return Err(McpError::Timeout(timeout)),
                Err(RecvTimeoutError::Disconnected) => return Err(McpError::Disconnected),
            }
        }
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        // Close stdin first so the child sees EOF and exits gracefully.
        drop(self.stdin.take());
        // Best-effort wait with a small deadline; kill if it doesn't exit.
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pump_stdout(mut stdout: impl Read, tx: Sender<String>) {
    let mut buf = [0u8; 8192];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                if tx.send(chunk).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}
