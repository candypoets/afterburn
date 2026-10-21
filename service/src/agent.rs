//! Ensure a workspace has an agent working on its map, and prompt it.
//!
//! Init is allowed to fail in this stage without failing the whole request: the document already
//! exists by the time an agent is touched, so the caller gets the link plus an `agent` object with
//! `state: "error"` and a reason.
//!
//! Policy, ported from the reference and the project README:
//! * agents already in the workspace are prompted, not duplicated;
//! * with no agent, the first installed CLI in a fixed order is started in a new background tab;
//! * a prompt asks the agent to read the mindmap skill and map the workspace checkout.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::herdr::{Agent, Herdr};
use crate::project::WorkspaceView;

/// Agent CLIs detected in this order when a workspace has none.
pub const AGENT_ORDER: [&str; 6] = ["codex", "claude", "kimi", "opencode", "gemini", "cursor"];

/// What happened to the agent part of Init.
#[derive(Clone, Debug, Serialize)]
pub struct AgentOutcome {
    /// `started`, `existing`, or `error`.
    pub state: AgentState,
    /// Agent kind, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Herdr agent name, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Pane the agent lives in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// Tab that was created or reused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// Whether a prompt was accepted.
    #[serde(skip_serializing_if = "is_false")]
    pub prompted: bool,
    /// Why agent setup failed. Only present when `state` is `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The agent lifecycle result exposed by the API.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// A new CLI was started and prompted.
    Started,
    /// An existing agent was prompted.
    Existing,
    /// The document exists but agent setup failed.
    Error,
}

impl AgentOutcome {
    /// A partial-failure result wrapping the document that was already published.
    pub fn error(reason: impl std::fmt::Display) -> Self {
        Self {
            state: AgentState::Error,
            kind: None,
            name: None,
            pane: None,
            tab: None,
            prompted: false,
            reason: Some(reason.to_string()),
        }
    }
}

/// Ensure an agent exists in `workspace` and submit the map prompt.
///
/// The returned outcome is always serializable; failures are reported in-band so Init can still
/// return the published document.
pub fn ensure(
    config: &Config,
    herdr: &Herdr,
    workspace: &WorkspaceView,
    file: Option<&str>,
) -> AgentOutcome {
    match ensure_inner(config, herdr, workspace, file) {
        Ok(outcome) => outcome,
        Err(error) => AgentOutcome::error(error),
    }
}

fn ensure_inner(
    config: &Config,
    herdr: &Herdr,
    workspace: &WorkspaceView,
    file: Option<&str>,
) -> Result<AgentOutcome> {
    let path = workspace.path.as_deref().ok_or_else(|| {
        Error::new(format!(
            "workspace {} has no checkout path; refusing to start or prompt an agent",
            workspace.id
        ))
    })?;

    let all_agents = herdr.agents()?;
    let workspace_agents: Vec<&Agent> = all_agents
        .iter()
        .filter(|agent| agent.workspace_id == workspace.id)
        .collect();
    let prompt = prompt_text(config, workspace, path, file);

    if let Some(agent) = pick_agent(&workspace_agents) {
        let pane = agent.pane_id.as_deref().ok_or_else(|| {
            Error::new(format!(
                "agent {} in workspace {} has no pane id",
                agent
                    .agent
                    .as_deref()
                    .or(agent.name.as_deref())
                    .unwrap_or("unknown"),
                workspace.id
            ))
        })?;
        prompt_when_ready(herdr, pane, &prompt)?;
        return Ok(AgentOutcome {
            state: AgentState::Existing,
            kind: agent.agent.clone(),
            name: agent.name.clone(),
            pane: Some(pane.to_string()),
            tab: agent.tab_id.clone(),
            prompted: true,
            reason: None,
        });
    }

    let kind = detect_kind().ok_or_else(|| {
        Error::new(format!(
            "no supported agent CLI found on PATH (tried {})",
            AGENT_ORDER.join(", ")
        ))
    })?;
    let created = herdr.create_tab(&workspace.id, path)?;
    let name = unique_agent_name(&all_agents, "mindmap")?;
    if let Err(error) = herdr.start_agent(&name, &kind, &created.pane_id) {
        // A first run in a folder asks the agent to trust it; the agent is alive but blocked,
        // so `agent start` reports it as not ready. Accept the default and carry on.
        if !accept_trust(herdr, &created.pane_id)? {
            return Err(error);
        }
    }
    prompt_when_ready(herdr, &created.pane_id, &prompt)?;

    Ok(AgentOutcome {
        state: AgentState::Started,
        kind: Some(kind),
        name: Some(name),
        pane: Some(created.pane_id),
        tab: created.tab_id,
        prompted: true,
        reason: None,
    })
}

fn pick_agent<'a>(agents: &[&'a Agent]) -> Option<&'a Agent> {
    agents
        .iter()
        .copied()
        .find(|agent| agent.focused && agent.pane_id.is_some())
        .or_else(|| {
            agents.iter().copied().find(|agent| {
                agent.pane_id.is_some() && agent.agent_status.as_deref() != Some("blocked")
            })
        })
        .or_else(|| agents.iter().copied().find(|agent| agent.pane_id.is_some()))
}

/// Accept a first-run folder-trust prompt, if one is on screen.
///
/// Codex (and similar CLIs) block on "Trust this folder?" in a new checkout. Herdr reports the
/// agent as blocked, and a prompt is rejected. The default choice is "Trust and continue", so
/// Enter moves it on; we wait briefly for the screen to settle before prompting.
fn accept_trust(herdr: &Herdr, pane: &str) -> Result<bool> {
    let screen = herdr.read_screen(pane).unwrap_or_default();
    let trust = screen.contains("Trust this folder")
        || screen.contains("enter continue")
        || screen.contains("Trust and continue");
    if !trust {
        return Ok(false);
    }
    herdr.send_keys(pane, &["enter"])?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(true)
}

/// Prompt the agent, accepting a trust prompt once if the first submission is rejected.
fn prompt_when_ready(herdr: &Herdr, pane: &str, prompt: &str) -> Result<()> {
    match herdr.prompt_agent(pane, prompt) {
        Ok(()) => Ok(()),
        Err(error) => {
            if accept_trust(herdr, pane)? {
                herdr.prompt_agent(pane, prompt)
            } else {
                Err(error)
            }
        }
    }
}

/// Find the first supported agent executable on `PATH`, in `AGENT_ORDER`.
fn detect_kind() -> Option<String> {
    AGENT_ORDER
        .iter()
        .find(|kind| find_in_path(kind).is_some())
        .map(|kind| (*kind).to_string())
}

/// A `which`-equivalent over `PATH` without spawning a process.
fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn unique_agent_name(agents: &[Agent], base: &str) -> Result<String> {
    let used: BTreeSet<&str> = agents
        .iter()
        .filter_map(|agent| agent.name.as_deref())
        .collect();
    if !used.contains(base) {
        return Ok(base.to_string());
    }
    for suffix in 2..=99 {
        let candidate = format!("{base}-{suffix}");
        if !used.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(Error::new(format!(
        "all agent names resembling {base:?} are already in use"
    )))
}

fn prompt_text(
    config: &Config,
    workspace: &WorkspaceView,
    path: &str,
    file: Option<&str>,
) -> String {
    let document = match file {
        Some(file) => format!("The project document file id is {file}."),
        None => "No project document file id is recorded yet.".to_string(),
    };
    format!(
        "Read {} and map {path} into the Mindmap for the {} workspace. {document} \
         Follow the skill: write the model to {path}/.mindmap/model.json, render it into the \
         project document, and start with the Overview (6–12 areas, every card sourced). \
         Work in {path}; do not draw directly on the canvas. When the Overview is rendered, \
         report what you mapped.",
        config.skill_file.display(),
        workspace.label,
    )
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(path: Option<&str>) -> WorkspaceView {
        WorkspaceView {
            id: "wP".into(),
            label: "eisledger".into(),
            path: path.map(str::to_owned),
            repo: Some("eisledger".into()),
            worktree: false,
            agents: 0,
            agent_names: Vec::new(),
            agent_status: "unknown".into(),
            focused: false,
            map: "empty".into(),
            file: None,
            tabs: Some(1),
            panes: Some(1),
        }
    }

    #[test]
    fn prompt_names_skill_path_and_file_id() {
        let config = Config {
            data_dir: PathBuf::from("/repo/service/data"),
            legacy_records: PathBuf::from("/repo/project/data/projects.json"),
            empty_template: PathBuf::from("/repo/service/assets/empty.figcraft"),
            key_file: PathBuf::from("/repo/server/data/server.key"),
            figcraft_cli: PathBuf::from("/repo/figcraft-cli"),
            herdr_bin: PathBuf::from("herdr"),
            public_server: "host:1".into(),
            local_server: "ws://localhost:1/".into(),
            bind: "0.0.0.0:1".into(),
            skill_file: PathBuf::from("/repo/skills/mindmap/SKILL.md"),
        };
        let prompt = prompt_text(
            &config,
            &workspace(Some("/repo/eisledger")),
            "/repo/eisledger",
            Some("3e3bb408a90e35a1"),
        );
        assert!(prompt.contains("/repo/skills/mindmap/SKILL.md"));
        assert!(prompt.contains("map /repo/eisledger"));
        assert!(prompt.contains("3e3bb408a90e35a1"));
    }

    #[test]
    fn error_outcome_is_link_safe_and_has_no_prompted_field() {
        let value =
            serde_json::to_value(AgentOutcome::error("no path")).unwrap_or(serde_json::Value::Null);
        assert_eq!(
            value,
            serde_json::json!({ "state": "error", "reason": "no path" })
        );
    }

    #[test]
    fn unique_names_skip_live_ones() {
        let agents = vec![Agent {
            workspace_id: "wP".into(),
            agent: Some("codex".into()),
            name: Some("mindmap".into()),
            pane_id: Some("wP:p1".into()),
            tab_id: Some("wP:t1".into()),
            agent_status: Some("idle".into()),
            focused: false,
        }];
        assert_eq!(
            unique_agent_name(&agents, "mindmap").unwrap_or_default(),
            "mindmap-2"
        );
        assert_eq!(
            unique_agent_name(&[], "mindmap").unwrap_or_default(),
            "mindmap"
        );
    }
}
