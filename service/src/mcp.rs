//! A small MCP client for `figcraft-cli mcp`.
//!
//! The protocol is newline-delimited JSON-RPC over stdin/stdout. Responses are read on a helper
//! thread so a wedged CLI cannot block a service thread forever. Only the calls the project
//! service needs are implemented: initialize, one document open, `collab.publish`, and
//! `collab.peers`.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result, ResultExt};

const RPC_TIMEOUT: Duration = Duration::from_secs(60);
const TOOL_TEXT_LIMIT: usize = 700;

/// One live MCP session with the FigCraft CLI.
///
/// Dropping the session kills the child and joins the reader thread. That is intentional: the
/// service has no long-lived MCP connection, it opens one per publish and releases it.
pub struct FigcraftSession {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<std::result::Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
}

impl FigcraftSession {
    /// Spawn `figcraft-cli mcp` and complete the MCP handshake.
    pub fn start(cli: &Path) -> Result<Self> {
        if !cli.is_file() {
            return Err(Error::new(format!(
                "figcraft CLI not found at {}",
                cli.display()
            )));
        }
        let mut child = Command::new(cli)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Keep CLI diagnostics in the service's stderr; don't let a full pipe deadlock it.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::new(format!("could not start {} mcp: {error}", cli.display()))
            })?;

        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new("figcraft MCP has no stdin"));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new("figcraft MCP has no stdout"));
            }
        };

        let (sender, responses) = mpsc::channel();
        let reader = match thread::Builder::new()
            .name("figcraft-mcp-stdout".to_string())
            .spawn(move || read_responses(stdout, sender))
        {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new(format!(
                    "could not start the figcraft MCP reader thread: {error}"
                )));
            }
        };

        let mut session = Self {
            child,
            stdin,
            responses,
            reader: Some(reader),
            next_id: 0,
        };
        session.initialize()?;
        Ok(session)
    }

    /// Execute one FigCraft command through the `run_command` tool.
    pub fn run_command(&mut self, command: &str, params: Value) -> Result<Value> {
        self.call_tool(
            "run_command",
            json!({ "command": command, "params": params }),
        )
    }

    /// Call an MCP tool and parse its text payload as JSON.
    ///
    /// Tool errors (`isError`) become `Err`; non-JSON text is returned as a JSON string, which is
    /// what the Python reference did for commands such as `list_commands`.
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let result = self.rpc(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        let text = result
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_error {
            let detail = if text.is_empty() {
                "no detail".to_string()
            } else {
                excerpt(text, TOOL_TEXT_LIMIT)
            };
            return Err(Error::new(format!("{name} failed: {detail}")));
        }
        if text.is_empty() {
            if let Some(content) = result.get("content") {
                return Ok(content.clone());
            }
            return Err(Error::new(format!("{name} returned no content")));
        }
        match serde_json::from_str::<Value>(text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(text.to_string())),
        }
    }

    fn initialize(&mut self) -> Result<()> {
        self.rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {
                    "name": "afterburn-service",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;
        self.notify("notifications/initialized")
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(format!(
                    "figcraft MCP timed out waiting for {method}"
                )));
            }
            match self.responses.recv_timeout(remaining) {
                Ok(Ok(message)) => {
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        // A notification or a stale reply; keep waiting for ours.
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        return Err(Error::new(format!(
                            "figcraft MCP {method} failed: {}",
                            excerpt(&error.to_string(), TOOL_TEXT_LIMIT)
                        )));
                    }
                    return message.get("result").cloned().ok_or_else(|| {
                        Error::new(format!("figcraft MCP {method} returned no result"))
                    });
                }
                Ok(Err(error)) => return Err(Error::new(error)),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(Error::new(format!(
                        "figcraft MCP timed out waiting for {method}"
                    )))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::new(
                        "figcraft MCP closed the connection before replying",
                    ))
                }
            }
        }
    }

    fn notify(&mut self, method: &str) -> Result<()> {
        self.write_message(&json!({ "jsonrpc": "2.0", "method": method }))
    }

    fn write_message(&mut self, message: &Value) -> Result<()> {
        let mut encoded = serde_json::to_vec(message)?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .context("write to figcraft MCP")?;
        self.stdin.flush().context("flush figcraft MCP")?;
        Ok(())
    }
}

impl Drop for FigcraftSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn read_responses(
    stdout: std::process::ChildStdout,
    sender: mpsc::Sender<std::result::Result<Value, String>>,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        match line {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(value) => {
                        if sender.send(Ok(value)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ =
                            sender.send(Err(format!("figcraft MCP sent invalid JSON: {error}")));
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(Err(format!("read from figcraft MCP: {error}")));
                return;
            }
        }
    }
}

fn excerpt(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit).collect();
    shortened.push('…');
    shortened
}
