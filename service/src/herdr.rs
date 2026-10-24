//! A thin typed client for the shared Herdr server's socket API.
//!
//! Herdr is the source of truth for workspaces, agents, tabs and panes. We only read JSON and
//! issue the few mutations Init needs, as the same requests the `herdr` CLI sends, straight to
//! the live session's socket.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result, ResultExt};

/// A Herdr workspace as returned by `workspace.list`.
#[derive(Clone, Debug, Deserialize)]
pub struct Workspace {
    /// Stable workspace id (`wP`).
    pub workspace_id: String,
    /// Human label shown in the inventory.
    pub label: String,
    /// The most recently seen agent state for the workspace.
    #[serde(default)]
    pub agent_status: Option<String>,
    /// Whether the workspace is focused in the user's Herdr UI.
    #[serde(default)]
    pub focused: bool,
    /// Number of tabs.
    #[serde(default)]
    pub tab_count: Option<u64>,
    /// Number of panes.
    #[serde(default)]
    pub pane_count: Option<u64>,
    /// Git worktree information, when Herdr knows a checkout.
    #[serde(default)]
    pub worktree: Option<Worktree>,
}

/// The `worktree` object in a Herdr workspace.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Worktree {
    /// Absolute checkout path.
    #[serde(default)]
    pub checkout_path: Option<String>,
    /// Repository name (`repo_name`).
    #[serde(default)]
    pub repo_name: Option<String>,
    /// True when this checkout is a linked git worktree.
    #[serde(default)]
    pub is_linked_worktree: Option<bool>,
}

/// One recognized agent from `agent.list`.
#[derive(Clone, Debug, Deserialize)]
pub struct Agent {
    /// Workspace the agent lives in.
    pub workspace_id: String,
    /// Agent kind (`codex`, `claude`, …); might be absent for an unknown occupant.
    #[serde(default)]
    pub agent: Option<String>,
    /// Optional unique agent name.
    #[serde(default)]
    pub name: Option<String>,
    /// Pane hosting the agent.
    #[serde(default)]
    pub pane_id: Option<String>,
    /// Tab hosting the agent.
    #[serde(default)]
    pub tab_id: Option<String>,
    /// Lifecycle state (`idle`, `working`, `blocked`, `done`, `unknown`).
    #[serde(default)]
    pub agent_status: Option<String>,
    /// Focused in the user's UI.
    #[serde(default)]
    pub focused: bool,
    /// The pane's terminal title without spinners or styling: what the agent is doing.
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
}

/// One pane from `pane.list`.
#[derive(Clone, Debug, Deserialize)]
pub struct Pane {
    /// Stable pane id (`wX:p2`).
    pub pane_id: String,
    /// Tab hosting the pane.
    #[serde(default)]
    pub tab_id: Option<String>,
    /// Working directory of the pane's shell.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Directory of the foreground process, when Herdr knows it.
    ///
    /// Parsed with the rest of the pane payload; the path fallback uses `cwd` on purpose.
    #[serde(default)]
    #[allow(dead_code)]
    pub foreground_cwd: Option<String>,
    /// Focused in the user's UI.
    #[serde(default)]
    pub focused: bool,
}

#[derive(Debug, Deserialize)]
struct AgentList {
    #[serde(default)]
    agents: Vec<Agent>,
}

#[derive(Debug, Deserialize)]
struct PaneList {
    #[serde(default)]
    panes: Vec<Pane>,
}

/// The result of `tab.create`, normalized across the possible response shapes.
#[derive(Clone, Debug)]
pub struct CreatedTab {
    /// The new tab id, when Herdr returned one.
    pub tab_id: Option<String>,
    /// The pane to start the agent in.
    pub pane_id: String,
}

/// A client for the shared Herdr server's socket API.
///
/// Every `herdr` CLI command is one JSON request to this socket, so the node sends the requests
/// itself instead of spawning the CLI for each call.
#[derive(Clone, Debug)]
pub struct Herdr {
    socket: PathBuf,
}

/// Reads, lists and plain mutations answer at once.
const QUICK: Duration = Duration::from_secs(15);
/// `agent.start` waits for the agent to be ready (Herdr's default startup timeout is 30 s).
const START: Duration = Duration::from_secs(45);

impl Herdr {
    /// Talk to the Herdr server listening on `socket`.
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    /// List all recognized agents in the session.
    pub fn agents(&self) -> Result<Vec<Agent>> {
        let value = self.call("agent.list", json!({}), QUICK)?;
        let parsed: AgentList = serde_json::from_value(value).context("parse Herdr agent.list")?;
        Ok(parsed.agents)
    }

    /// List the panes of one workspace.
    ///
    /// Used for the cwd fallback (a workspace with no `worktree` still has panes, each with a
    /// `cwd`) and for finding the pane of a freshly created tab.
    pub fn panes(&self, workspace_id: &str) -> Result<Vec<Pane>> {
        let value = self.call("pane.list", json!({"workspace_id": workspace_id}), QUICK)?;
        let parsed: PaneList = serde_json::from_value(value)
            .context(format!("parse Herdr pane.list for {workspace_id}"))?;
        Ok(parsed.panes)
    }

    /// Create a background tab in `workspace_id` at `cwd`, and resolve its first pane.
    pub fn create_tab(&self, workspace_id: &str, cwd: &str) -> Result<CreatedTab> {
        let value = self.call(
            "tab.create",
            json!({"workspace_id": workspace_id, "cwd": cwd, "label": "mindmap", "focus": false}),
            QUICK,
        )?;

        let tab_id = string_at(&value, "/tab/tab_id")
            .or_else(|| string_at(&value, "/tab_id"))
            .or_else(|| value.get("tab").and_then(Value::as_str).map(str::to_owned));
        let mut pane_id = string_at(&value, "/root_pane/pane_id")
            .or_else(|| string_at(&value, "/pane_id"))
            .or_else(|| string_at(&value, "/pane/pane_id"));

        // Older/newer servers may omit the pane from the create response. The tab id is enough
        // to find it again in the pane inventory.
        if pane_id.is_none() {
            if let Some(tab) = tab_id.as_deref() {
                if let Some(pane) = self.pane_for_tab(workspace_id, tab)? {
                    pane_id = Some(pane.pane_id);
                }
            }
        }

        let pane_id = pane_id.ok_or_else(|| {
            Error::new(format!(
                "Herdr tab.create returned no pane id: {}",
                excerpt(&value.to_string(), 400)
            ))
        })?;
        Ok(CreatedTab { tab_id, pane_id })
    }

    /// Start a supported agent in a pane that is at a shell prompt.
    pub fn start_agent(&self, name: &str, kind: &str, pane_id: &str) -> Result<()> {
        self.call(
            "agent.start",
            json!({"name": name, "kind": kind, "pane_id": pane_id}),
            START,
        )?;
        Ok(())
    }

    /// The agent's visible screen as plain text, for first-run prompts (folder trust).
    pub fn read_screen(&self, pane_id: &str) -> Result<String> {
        let value = self.call(
            "agent.read",
            json!({"target": pane_id, "source": "visible", "format": "text"}),
            QUICK,
        )?;
        Ok(string_at(&value, "/read/text").unwrap_or_default())
    }

    /// Type literal text into a pane and press Enter (what `herdr pane run` sends). This is the
    /// fallback for terminal agents Herdr detects but cannot prompt through its agent protocol
    /// (dsh-tui).
    pub fn pane_run(&self, pane_id: &str, text: &str) -> Result<()> {
        self.call(
            "pane.send_input",
            json!({"pane_id": pane_id, "text": text, "keys": ["Enter"]}),
            QUICK,
        )?;
        Ok(())
    }

    /// Press keys in an agent's pane (Herdr names: `enter`, `esc`, single characters).
    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        self.call(
            "agent.send_keys",
            json!({"target": pane_id, "keys": keys}),
            QUICK,
        )?;
        Ok(())
    }

    /// Submit one prompt to the agent in `pane_id`.
    ///
    /// No wait: Init returns once the prompt is submitted, not when the agent has finished a
    /// potentially long map-building turn. Herdr's `agent.prompt` validates blocked state and
    /// submits the text with the pane's current paste/Enter semantics.
    pub fn prompt_agent(&self, pane_id: &str, prompt: &str) -> Result<()> {
        self.call(
            "agent.prompt",
            json!({"target": pane_id, "text": prompt}),
            QUICK,
        )?;
        Ok(())
    }

    fn pane_for_tab(&self, workspace_id: &str, tab_id: &str) -> Result<Option<Pane>> {
        Ok(self
            .panes(workspace_id)?
            .into_iter()
            .find(|pane| pane.tab_id.as_deref() == Some(tab_id)))
    }

    /// Send one request and return its `result`.
    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        crate::node_herdr::rpc(&self.socket, method, params, timeout)
            .map_err(|error| Error::new(format!("herdr {method} failed: {}", error)))
    }
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn excerpt(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Run inside a Herdr pane with `cargo test -p mindmap-node herdr::live -- --ignored`.
    #[test]
    #[ignore = "needs a running Herdr server"]
    fn lists_agents_and_panes_and_reads_a_screen_over_the_socket() {
        let socket = std::env::var_os("HERDR_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config/herdr/herdr.sock")
            });
        let herdr = Herdr::new(socket);
        let agents = herdr.agents().unwrap();
        assert!(!agents.is_empty());
        let workspace = std::env::var("HERDR_WORKSPACE_ID").unwrap();
        let panes = herdr.panes(&workspace).unwrap();
        assert!(!panes.is_empty());
        let pane = std::env::var("HERDR_PANE_ID").unwrap();
        assert!(!herdr.read_screen(&pane).unwrap().trim().is_empty());
        let error = herdr
            .send_keys("w-missing:p-missing", &["esc"])
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("herdr agent.send_keys failed"),
            "{error}"
        );
    }
}
