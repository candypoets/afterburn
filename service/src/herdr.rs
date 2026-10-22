//! A thin typed wrapper over the `herdr` CLI.
//!
//! Herdr is the source of truth for workspaces, agents, tabs and panes. We only read JSON and
//! issue the few mutations Init needs. Every call runs with `HERDR_ENV=1` and talks to the live
//! session; the service never starts or stops a Herdr server.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result, ResultExt};

/// A Herdr workspace as returned by `herdr workspace list`.
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

/// One recognized agent from `herdr agent list`.
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
}

/// One pane from `herdr pane list`.
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
struct WorkspaceList {
    #[serde(default)]
    workspaces: Vec<Workspace>,
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

/// The result of `herdr tab create`, normalized across the possible response shapes.
#[derive(Clone, Debug)]
pub struct CreatedTab {
    /// The new tab id, when Herdr returned one.
    pub tab_id: Option<String>,
    /// The pane to start the agent in.
    pub pane_id: String,
}

/// The `herdr` binary plus its environment.
#[derive(Clone, Debug)]
pub struct Herdr {
    bin: PathBuf,
}

impl Herdr {
    /// Wrap the configured binary.
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// List workspaces.
    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let value = self.call(&["workspace", "list"])?;
        let parsed: WorkspaceList =
            serde_json::from_value(value).context("parse `herdr workspace list`")?;
        Ok(parsed.workspaces)
    }

    /// List all recognized agents in the session.
    pub fn agents(&self) -> Result<Vec<Agent>> {
        let value = self.call(&["agent", "list"])?;
        let parsed: AgentList =
            serde_json::from_value(value).context("parse `herdr agent list`")?;
        Ok(parsed.agents)
    }

    /// List the panes of one workspace.
    ///
    /// Used for the cwd fallback (a workspace with no `worktree` still has panes, each with a
    /// `cwd`) and for finding the pane of a freshly created tab.
    pub fn panes(&self, workspace_id: &str) -> Result<Vec<Pane>> {
        let value = self.call(&["pane", "list", "--workspace", workspace_id])?;
        let parsed: PaneList = serde_json::from_value(value).context(format!(
            "parse `herdr pane list --workspace {workspace_id}`"
        ))?;
        Ok(parsed.panes)
    }

    /// Create a background tab in `workspace_id` at `cwd`, and resolve its first pane.
    pub fn create_tab(&self, workspace_id: &str, cwd: &str) -> Result<CreatedTab> {
        let value = self.call(&[
            "tab",
            "create",
            "--workspace",
            workspace_id,
            "--cwd",
            cwd,
            "--label",
            "mindmap",
            "--no-focus",
        ])?;

        let tab_id = string_at(&value, "/tab/tab_id")
            .or_else(|| string_at(&value, "/tab_id"))
            .or_else(|| value.get("tab").and_then(Value::as_str).map(str::to_owned));
        let mut pane_id = string_at(&value, "/root_pane/pane_id")
            .or_else(|| string_at(&value, "/pane_id"))
            .or_else(|| string_at(&value, "/pane/pane_id"));

        // Older/newer CLI wrappers may omit the pane from the create response. The tab id is
        // enough to find it again in the pane inventory.
        if pane_id.is_none() {
            if let Some(tab) = tab_id.as_deref() {
                if let Some(pane) = self.pane_for_tab(workspace_id, tab)? {
                    pane_id = Some(pane.pane_id);
                }
            }
        }

        let pane_id = pane_id.ok_or_else(|| {
            Error::new(format!(
                "`herdr tab create` returned no pane id: {}",
                excerpt(&value.to_string(), 400)
            ))
        })?;
        Ok(CreatedTab { tab_id, pane_id })
    }

    /// Start a supported agent in a pane that is at a shell prompt.
    pub fn start_agent(&self, name: &str, kind: &str, pane_id: &str) -> Result<()> {
        self.call(&["agent", "start", name, "--kind", kind, "--pane", pane_id])?;
        Ok(())
    }

    /// The agent's visible screen as plain text, for first-run prompts (folder trust).
    ///
    /// `herdr agent read` prints text rather than the usual JSON envelope, so this bypasses
    /// [`Self::call`] and returns stdout directly.
    pub fn read_screen(&self, pane_id: &str) -> Result<String> {
        let output = Command::new(&self.bin)
            .args([
                "agent", "read", pane_id, "--source", "visible", "--format", "text",
            ])
            .env("HERDR_ENV", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                Error::new(format!(
                    "could not run {}: {error}",
                    self.bin.to_string_lossy()
                ))
            })?;
        if !output.status.success() {
            return Err(Error::new(herdr_error(
                &output.stderr,
                output.status.code(),
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Type literal text into a pane and press Enter (`herdr pane run`). This is the fallback for
    /// terminal agents Herdr detects but cannot prompt through its agent protocol (dsh-tui).
    pub fn pane_run(&self, pane_id: &str, text: &str) -> Result<()> {
        // `pane run` prints nothing on success, unlike the JSON-returning commands.
        let output = Command::new(&self.bin)
            .args(["pane", "run", pane_id, text])
            .env("HERDR_ENV", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                Error::new(format!(
                    "could not run {}: {error}",
                    self.bin.to_string_lossy()
                ))
            })?;
        if !output.status.success() {
            return Err(Error::new(herdr_error(
                &output.stderr,
                output.status.code(),
            )));
        }
        Ok(())
    }

    /// Press keys in an agent's pane (Herdr names: `enter`, `esc`, single characters).
    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let mut args = vec!["agent", "send-keys", pane_id];
        args.extend_from_slice(keys);
        self.call(&args)?;
        Ok(())
    }

    /// Submit one prompt to the agent in `pane_id`.
    ///
    /// No `--wait`: Init returns once the prompt is submitted, not when the agent has finished a
    /// potentially long map-building turn. `herdr agent prompt` validates blocked state and
    /// submits the text with the pane's current paste/Enter semantics.
    pub fn prompt_agent(&self, pane_id: &str, prompt: &str) -> Result<()> {
        self.call(&["agent", "prompt", pane_id, prompt])?;
        Ok(())
    }

    fn pane_for_tab(&self, workspace_id: &str, tab_id: &str) -> Result<Option<Pane>> {
        Ok(self
            .panes(workspace_id)?
            .into_iter()
            .find(|pane| pane.tab_id.as_deref() == Some(tab_id)))
    }

    /// Run one Herdr command and return the JSON `result` field.
    fn call(&self, args: &[&str]) -> Result<Value> {
        let output = Command::new(&self.bin)
            .args(args)
            .env("HERDR_ENV", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                Error::new(format!(
                    "could not run {}: {error}",
                    self.bin.to_string_lossy()
                ))
            })?;

        let label = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        if !output.status.success() {
            return Err(Error::new(format!(
                "herdr {label} failed: {}",
                herdr_error(&output.stderr, output.status.code())
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: Value =
            serde_json::from_str(stdout.trim()).context(format!("parse `herdr {label}` output"))?;
        value.get("result").cloned().ok_or_else(|| {
            Error::new(format!(
                "`herdr {label}` returned no result: {}",
                excerpt(stdout.trim(), 400)
            ))
        })
    }
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn herdr_error(stderr: &[u8], status: Option<i32>) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let trimmed = stderr.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .unwrap_or("unknown error");
            let code = error.get("code").and_then(Value::as_str);
            return match code {
                Some(code) if !code.is_empty() => format!("{message} ({code})"),
                _ => message.to_string(),
            };
        }
    }
    if trimmed.is_empty() {
        match status {
            Some(status) => format!("exit status {status}"),
            None => "terminated by a signal".to_string(),
        }
    } else {
        excerpt(trimmed, 600)
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
