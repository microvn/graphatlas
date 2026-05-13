//! Minimal MCP server stand-in for `McpChild` integration tests. Reads
//! JSON-RPC requests line-by-line on stdin, echoes a canned response on
//! stdout with `result: { echoed: <method> }`. Not a real MCP server —
//! just enough to prove spawn + handshake + tools_call plumbing.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req): Result<Value, _> = serde_json::from_str(&line) else {
            continue;
        };
        // Notifications carry no id — nothing to respond with.
        let Some(id) = req.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "echoed": method }
        });
        if writeln!(out, "{}", response).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
    }
}
