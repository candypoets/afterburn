//! The node's client of the machine's shared Herdr session.
//!
//! No private session, project store, owner URL, or server key is involved. CLI operations
//! inherit Herdr's session/socket configuration, just like the saved machine in the Mac UI.
//! Socket events are a separate local connection from the node's public multiplexed transport.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::{self, AgentOutcome, Target};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::herdr::{Agent, Herdr, Pane, Workspace};
use crate::workspace::{AgentDetail, LastSync, WorkspaceView};

#[path = "node_herdr_events.rs"]
mod events;
pub use events::{HerdrEvent, HerdrMonitor};

const OFFICIAL_INSTALLER: &str = "https://herdr.dev/install.sh";

/// Lifecycle metadata only; terminal text and environment variables never enter node events.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentStatus {
    pub workspace_id: String,
    pub pane_id: String,
    pub status: String,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub tab_id: Option<String>,
}

/// Safe to expose through host.status and host.ensure.
#[derive(Clone, Debug, Serialize)]
pub struct HostStatus {
    pub installed: bool,
    pub running: bool,
    pub approval_required: bool,
    pub binary: Option<PathBuf>,
    pub socket: PathBuf,
    pub session: String,
}

/// Explicit repository context is mandatory; never use the UI's focused workspace.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreateWorktree {
    pub workspace_id: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub path: Option<String>,
    pub label: Option<String>,
    #[serde(default)]
    pub trust_repository: bool,
}

/// Cheaply cloneable handle. All clones serialize installation and agent creation.
#[derive(Clone, Debug)]
pub struct NodeHerdr {
    requested_binary: PathBuf,
    socket: PathBuf,
    session: String,
    setup: Arc<Mutex<()>>,
    prompts: Arc<Mutex<()>>,
}

impl NodeHerdr {
    /// Capture the current shared-session address without starting or installing anything.
    pub fn new(config: &Config) -> Result<Self> {
        let (socket, session) = current_socket()?;
        Ok(Self {
            requested_binary: config.herdr_bin.clone(),
            socket,
            session,
            setup: Arc::new(Mutex::new(())),
            prompts: Arc::new(Mutex::new(())),
        })
    }

    /// A read-only probe, including whether installation needs the user's approval.
    pub fn status(&self) -> Result<HostStatus> {
        let binary = self.binary();
        let running = events::rpc(&self.socket, "ping", json!({}), Duration::from_secs(2))
            .is_ok_and(|result| result["type"] == "pong");
        Ok(HostStatus {
            installed: binary.is_some(),
            approval_required: binary.is_none(),
            binary,
            running,
            socket: self.socket.clone(),
            session: self.session.clone(),
        })
    }

    /// Start installed Herdr headlessly; run its official installer only with explicit approval.
    /// Missing Herdr without approval is a normal status result, not an installation attempt.
    /// Dropping the node never stops the shared Herdr server or its agents.
    pub fn ensure(&self, install_herdr: bool) -> Result<HostStatus> {
        let _guard = self
            .setup
            .lock()
            .map_err(|_| Error::new("Herdr setup lock poisoned"))?;
        let mut status = self.status()?;
        if !status.installed {
            if !install_herdr {
                return Ok(status);
            }
            install()?;
            status = self.status()?;
            if !status.installed {
                return Err(Error::new(
                    "Herdr installer finished but the configured binary is unavailable",
                ));
            }
        }
        if status.running {
            return Ok(status);
        }

        let binary = status
            .binary
            .as_ref()
            .ok_or_else(|| Error::new("Herdr is not installed"))?;
        // `herdr server` is the documented foreground headless entry point. Spawn it in its own
        // process group with detached standard streams. Reap it without owning its lifetime.
        let mut child = Command::new(binary)
            .arg("server")
            .env("HERDR_SOCKET_PATH", &self.socket)
            .env("HERDR_ENV", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| Error::new("could not start the shared Herdr headless server"))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            status = self.status()?;
            if status.running {
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(status);
            }
            if child.try_wait()?.is_some() {
                // A simultaneous UI/node startup may have won Herdr's server lock.
                status = self.status()?;
                if status.running {
                    return Ok(status);
                }
                return Err(Error::new(
                    "Herdr headless server exited before its socket became ready",
                ));
            }
            if Instant::now() >= deadline {
                thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(Error::new(
                    "Herdr headless server did not become ready within 15 seconds",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Inventory has the legacy WorkspaceView JSON shape, with map presence from the checkout.
    /// The session host may enrich `map`/`file` with its live-document state after this call.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceView>> {
        let snapshot = self.snapshot()?;
        let workspaces: Vec<Workspace> = serde_json::from_value(snapshot["workspaces"].clone())?;
        let agents: Vec<Agent> = serde_json::from_value(snapshot["agents"].clone())?;
        let clis = agent::installed_kinds();
        let mut views = Vec::with_capacity(workspaces.len());
        for workspace in workspaces {
            let panes: Vec<Pane> = snapshot["panes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|pane| pane["workspace_id"].as_str() == Some(&workspace.workspace_id))
                .map(|pane| serde_json::from_value(pane.clone()))
                .collect::<std::result::Result<_, _>>()?;
            views.push(workspace_view(workspace, &agents, &panes, &clis));
        }
        views.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(views)
    }

    pub fn find_workspace(&self, query: &str) -> Result<WorkspaceView> {
        resolve_workspace(self.workspaces()?, query)
    }

    pub fn preflight(&self, workspace: &WorkspaceView, target: &Target) -> Result<()> {
        agent::preflight(&self.herdr()?, workspace, target)
    }

    /// Uses the existing selection, new-tab, first-run, and prompt helpers. Prompt text is never
    /// logged here. The caller supplies node/session instructions, not legacy owner links.
    pub fn prompt(
        &self,
        workspace: &WorkspaceView,
        target: &Target,
        text: &str,
    ) -> Result<AgentOutcome> {
        if text.trim().is_empty() {
            return Err(Error::new("agent prompt must not be empty"));
        }
        let _guard = self
            .prompts
            .lock()
            .map_err(|_| Error::new("Herdr prompt lock poisoned"))?;
        let herdr = self.herdr()?;
        agent::preflight(&herdr, workspace, target)?;
        Ok(agent::ensure_with_prompt(&herdr, workspace, target, text))
    }

    /// Herdr owns git/worktree creation and returns the created workspace/pane handles.
    pub fn create_worktree(&self, request: &CreateWorktree) -> Result<Value> {
        if [request.workspace_id.as_deref(), request.cwd.as_deref()]
            .into_iter()
            .flatten()
            .any(|context| context.trim().is_empty())
        {
            return Err(Error::new("worktree context must not be empty"));
        }
        if request
            .workspace_id
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            == request.cwd.as_deref().is_some_and(|s| !s.trim().is_empty())
        {
            return Err(Error::new(
                "worktree creation needs exactly one workspace_id or cwd",
            ));
        }
        let mut params = serde_json::to_value(request)?;
        params
            .as_object_mut()
            .ok_or_else(|| Error::new("invalid worktree request"))?
            .retain(|_, v| !v.is_null());
        params["focus"] = json!(false);
        events::rpc(
            &self.socket,
            "worktree.create",
            params,
            Duration::from_secs(120),
        )
    }

    /// Start a reconnecting socket subscription. Retain the returned guard for its lifetime.
    pub fn monitor(&self) -> Result<HerdrMonitor> {
        HerdrMonitor::start(self.clone())
    }

    fn snapshot(&self) -> Result<Value> {
        let result = events::rpc(
            &self.socket,
            "session.snapshot",
            json!({}),
            Duration::from_secs(10),
        )?;
        result
            .get("snapshot")
            .cloned()
            .ok_or_else(|| Error::new("Herdr session.snapshot omitted snapshot"))
    }

    fn herdr(&self) -> Result<Herdr> {
        // Existing helpers inherit the same environment. Reject process-wide reconfiguration
        // instead of accidentally sending an agent prompt into another session.
        if current_socket()?.0 != self.socket {
            return Err(Error::new(
                "Herdr session configuration changed; recreate the node Herdr client",
            ));
        }
        self.binary().map(Herdr::new).ok_or_else(|| {
            Error::new("Herdr is not installed; host.ensure requires installation approval")
        })
    }

    fn binary(&self) -> Option<PathBuf> {
        discover_binary(&self.requested_binary)
    }
}

fn current_socket() -> Result<(PathBuf, String)> {
    resolve_socket(
        env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from),
        env::var("HERDR_SESSION").ok().as_deref(),
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_socket(
    socket: Option<PathBuf>,
    session: Option<&str>,
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<(PathBuf, String)> {
    let root = config_home
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|root| root.join("herdr"));
    if let Some(socket) = socket {
        if socket.as_os_str().is_empty() {
            return Err(Error::new("HERDR_SOCKET_PATH must not be empty"));
        }
        // Managed panes inherit even the default socket as an explicit override. Report its
        // real session so the Mac can reuse the matching saved-machine profile.
        let session = root
            .as_ref()
            .and_then(|root| {
                if socket == root.join("herdr.sock") {
                    return Some("default".to_owned());
                }
                let relative = socket.strip_prefix(root.join("sessions")).ok()?;
                let mut components = relative.iter();
                let name = components.next()?.to_str()?;
                if components.next()? == "herdr.sock" && components.next().is_none() {
                    Some(name.to_owned())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "socket_override".into());
        return Ok((socket, session));
    }
    let session = session.unwrap_or("default");
    if session.is_empty()
        || matches!(session, "." | "..")
        || session.len() > 64
        || !session
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(Error::new("invalid HERDR_SESSION"));
    }
    let root = root.ok_or_else(|| {
        Error::new("HOME or XDG_CONFIG_HOME is required to find Herdr's shared socket")
    })?;
    let root = if session == "default" {
        root
    } else {
        root.join("sessions").join(session)
    };
    Ok((root.join("herdr.sock"), session.into()))
}

fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn path_binary(name: &Path) -> Option<PathBuf> {
    if name.components().count() > 1 {
        return executable(name).then(|| name.to_owned());
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| executable(path))
    })
}

fn discover_binary(requested: &Path) -> Option<PathBuf> {
    // Explicit configuration is authoritative; do not silently choose an unrelated installation.
    if requested != Path::new("herdr") {
        return path_binary(requested);
    }
    if let Some(bin) = env::var_os("HERDR_BIN_PATH")
        .map(PathBuf::from)
        .filter(|p| executable(p))
    {
        return Some(bin);
    }
    path_binary(requested).or_else(|| {
        let mut candidates = Vec::new();
        if let Some(home) = env::var_os("HOME") {
            candidates.push(PathBuf::from(&home).join(".local/bin/herdr"));
            candidates.push(PathBuf::from(home).join(".cargo/bin/herdr"));
        }
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin/herdr"),
            PathBuf::from("/usr/local/bin/herdr"),
        ]);
        candidates.into_iter().find(|p| executable(p))
    })
}

fn install() -> Result<()> {
    // Fetch into memory and feed sh on stdin; no script, credentials, or installer output files.
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "60",
            OFFICIAL_INSTALLER,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| Error::new("could not download the official Herdr installer"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(Error::new(
            "could not download the official Herdr installer",
        ));
    }
    let mut child = Command::new("sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| Error::new("installer stdin unavailable"))?
        .write_all(&output.stdout);
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(status) = child.try_wait()? {
            write_result?;
            return if status.success() {
                Ok(())
            } else {
                Err(Error::new("official Herdr installer failed"))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new("official Herdr installer timed out"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn workspace_view(
    workspace: Workspace,
    agents: &[Agent],
    panes: &[Pane],
    clis: &[String],
) -> WorkspaceView {
    let mine: Vec<_> = agents
        .iter()
        .filter(|a| a.workspace_id == workspace.workspace_id)
        .collect();
    let tree = workspace.worktree.as_ref();
    let path = tree
        .and_then(|tree| tree.checkout_path.clone())
        .filter(|p| !p.trim().is_empty())
        .or_else(|| pane_path(panes));
    let branch = path.as_deref().and_then(git_branch);
    let map = if path
        .as_ref()
        .is_some_and(|p| Path::new(p).join(".mindmap/map.figcraft").is_file())
    {
        "map"
    } else {
        "none"
    };
    let last_sync = if map == "map" {
        path.as_deref().and_then(last_sync)
    } else {
        None
    };
    WorkspaceView {
        id: workspace.workspace_id.clone(),
        label: workspace.label,
        path,
        repo: tree.and_then(|tree| tree.repo_name.clone()),
        worktree: tree
            .and_then(|tree| tree.is_linked_worktree)
            .unwrap_or(false),
        branch,
        agents: mine.len(),
        agent_names: mine
            .iter()
            .filter_map(|a| a.agent.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        agent_status: workspace.agent_status.unwrap_or_else(|| "unknown".into()),
        agents_detail: mine
            .iter()
            .filter_map(|a| {
                Some(AgentDetail {
                    pane: a.pane_id.clone()?,
                    kind: a.agent.clone(),
                    name: a.name.clone(),
                    status: a.agent_status.clone().unwrap_or_else(|| "unknown".into()),
                    focused: a.focused,
                    title: a
                        .terminal_title_stripped
                        .clone()
                        .map(|title| title.trim().to_owned())
                        .filter(|title| !title.is_empty()),
                })
            })
            .collect(),
        clis: clis.to_vec(),
        focused: workspace.focused,
        map: map.into(),
        tabs: workspace.tab_count,
        panes: workspace.pane_count,
        last_sync,
    }
}

fn pane_path(panes: &[Pane]) -> Option<String> {
    panes
        .iter()
        .filter(|p| p.focused)
        .chain(panes.iter())
        .filter_map(|p| p.cwd.as_ref())
        .find(|p| !p.trim().is_empty())
        .cloned()
}

/// The newest commit on HEAD carrying a `Mindmap-Sync:` trailer.
fn last_sync(path: &str) -> Option<LastSync> {
    let output = Command::new("git")
        .args([
            "-C",
            path,
            "log",
            "-1",
            "--regexp-ignore-case",
            "--grep=^Mindmap-Sync:",
            "--format=%h%x1f%s%x1f%cI",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut parts = text.trim().splitn(3, '\u{1f}');
    let commit = parts.next()?.trim().to_owned();
    let subject = parts.next()?.trim().to_owned();
    let at = parts.next()?.trim().to_owned();
    (!commit.is_empty()).then_some(LastSync {
        commit,
        subject,
        at,
    })
}

fn git_branch(path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", path, "symbolic-ref", "--quiet", "--short", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!branch.is_empty()).then_some(branch)
}

fn resolve_workspace(workspaces: Vec<WorkspaceView>, query: &str) -> Result<WorkspaceView> {
    if let Some(w) = workspaces.iter().find(|w| w.id == query) {
        return Ok(w.clone());
    }
    let exact: Vec<_> = workspaces.iter().filter(|w| w.label == query).collect();
    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    if exact.len() > 1 {
        return Err(Error::new("workspace label is ambiguous; use its id"));
    }
    let matches: Vec<_> = workspaces
        .into_iter()
        .filter(|w| {
            w.id.eq_ignore_ascii_case(query) || w.label.to_lowercase() == query.to_lowercase()
        })
        .collect();
    match matches.len() {
        1 => matches
            .into_iter()
            .next()
            .ok_or_else(|| Error::new("workspace not found")),
        0 => Err(Error::new("workspace not found")),
        _ => Err(Error::new("workspace label is ambiguous; use its id")),
    }
}

#[cfg(test)]
#[path = "node_herdr_tests.rs"]
mod tests;
