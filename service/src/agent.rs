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

/// Which agent Init should use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Prompt an existing agent, else start the first installed CLI.
    Auto,
    /// Prompt this exact pane, which must hold an agent in the workspace.
    Pane(String),
    /// Start a new agent of this kind in a new background tab.
    Kind(String),
}

impl Target {
    /// The canonical request form, echoed in [`AgentOutcome::target`].
    pub fn as_request(&self) -> String {
        match self {
            Target::Auto => "auto".to_string(),
            Target::Pane(pane) => format!("pane:{pane}"),
            Target::Kind(kind) => format!("kind:{kind}"),
        }
    }

    /// The agent kind named by this target, if any.
    pub fn kind(&self) -> Option<&str> {
        match self {
            Target::Kind(kind) => Some(kind),
            Target::Auto | Target::Pane(_) => None,
        }
    }

    /// The pane named by this target, if any.
    pub fn pane(&self) -> Option<&str> {
        match self {
            Target::Pane(pane) => Some(pane),
            Target::Auto | Target::Kind(_) => None,
        }
    }
}

/// Parse an Init target: absent or `auto`, `pane:<pane-id>`, or `kind:<agent-kind>`.
pub fn parse_target(raw: Option<&str>) -> Result<Target> {
    let raw = match raw {
        None => return Ok(Target::Auto),
        Some(raw) => raw.trim(),
    };
    if raw.is_empty() || raw.eq_ignore_ascii_case("auto") {
        return Ok(Target::Auto);
    }
    if let Some(pane) = raw.strip_prefix("pane:") {
        let pane = pane.trim();
        if pane.is_empty() {
            return Err(Error::new("\"pane:\" target needs a pane id"));
        }
        return Ok(Target::Pane(pane.to_string()));
    }
    if let Some(kind) = raw.strip_prefix("kind:") {
        let kind = kind.trim();
        if kind.is_empty() {
            return Err(Error::new("\"kind:\" target needs an agent kind"));
        }
        validate_kind(kind)?;
        return Ok(Target::Kind(kind.to_string()));
    }
    Err(Error::new(format!(
        "invalid target {raw:?}; expected \"auto\", \"pane:<pane-id>\", or \"kind:<agent-kind>\""
    )))
}

/// Reject kinds outside the supported set before any side effect.
fn validate_kind(kind: &str) -> Result<()> {
    if AGENT_ORDER.contains(&kind) {
        Ok(())
    } else {
        Err(Error::new(format!(
            "unsupported agent kind {kind:?} (supported: {})",
            AGENT_ORDER.join(", ")
        )))
    }
}

/// Check that a target can be honored in `workspace` without publishing or prompting anything.
///
/// The kind half is already covered by [`parse_target`]; repeating it here keeps callers that
/// construct a [`Target`] directly honest. The pane half needs Herdr's live agent list.
pub fn validate_target(herdr: &Herdr, workspace: &WorkspaceView, target: &Target) -> Result<()> {
    match target {
        Target::Auto => Ok(()),
        Target::Kind(kind) => validate_kind(kind),
        Target::Pane(pane) => {
            let agents = herdr.agents()?;
            let found = agents.iter().any(|agent| {
                agent.workspace_id == workspace.id
                    && agent.pane_id.as_deref() == Some(pane.as_str())
            });
            if found {
                Ok(())
            } else {
                Err(Error::new(format!(
                    "pane {pane} is not an agent in workspace {} ({})",
                    workspace.id, workspace.label
                )))
            }
        }
    }
}

/// Check that a target can be prompted for a Sync before any side effect.
///
/// [`validate_target`] covers the pane/kind syntax; a sync additionally needs a checkout path and
/// a source that can actually be prompted: an existing agent in the workspace for `auto`, or an
/// installed CLI for `auto` and `kind`. A failure here must be returned as a request error, before
/// a tab is created or a prompt is submitted.
pub fn preflight(herdr: &Herdr, workspace: &WorkspaceView, target: &Target) -> Result<()> {
    validate_target(herdr, workspace, target)?;
    let has_path = workspace
        .path
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty());
    if !has_path {
        return Err(Error::new(format!(
            "workspace {} ({}) has no checkout path; cannot prompt an agent",
            workspace.id, workspace.label
        )));
    }
    match target {
        Target::Auto => {
            let agents = herdr.agents()?;
            let existing = agents
                .iter()
                .any(|agent| agent.workspace_id == workspace.id && agent.pane_id.is_some());
            if existing || detect_kind().is_some() {
                Ok(())
            } else {
                Err(Error::new(format!(
                    "no supported agent CLI found on PATH (tried {})",
                    AGENT_ORDER.join(", ")
                )))
            }
        }
        Target::Kind(kind) => {
            if find_in_path(kind).is_some() {
                Ok(())
            } else {
                Err(Error::new(format!(
                    "agent CLI {kind:?} is not installed on PATH"
                )))
            }
        }
        Target::Pane(_) => Ok(()),
    }
}

/// What happened to the agent part of Init.
#[derive(Clone, Debug, Serialize)]
pub struct AgentOutcome {
    /// `started`, `existing`, or `error`.
    pub state: AgentState,
    /// Canonical target that was requested (`auto`, `pane:…`, `kind:…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
            target: None,
            kind: None,
            name: None,
            pane: None,
            tab: None,
            prompted: false,
            reason: Some(reason.to_string()),
        }
    }

    /// A partial-failure result that still says which pane or kind was requested.
    fn error_for(target: &Target, reason: impl std::fmt::Display) -> Self {
        let mut outcome = Self::error(reason);
        outcome.target = Some(target.as_request());
        outcome.kind = target.kind().map(str::to_owned);
        outcome.pane = target.pane().map(str::to_owned);
        outcome
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
    target: &Target,
) -> AgentOutcome {
    let prompt = workspace
        .path
        .as_deref()
        .map(|path| prompt_text(config, workspace, path, file));
    finish(herdr, workspace, target, prompt.as_deref())
}

/// Ensure an agent exists in `workspace` and submit a caller-supplied prompt.
///
/// This is the Send path: the caller has already rendered the comments and validated the target,
/// so no document is opened or published. Reusing an existing agent, starting the chosen kind,
/// first-run trust handling, and in-band failures are the same as [`ensure`].
pub fn ensure_with_prompt(
    herdr: &Herdr,
    workspace: &WorkspaceView,
    target: &Target,
    prompt: &str,
) -> AgentOutcome {
    finish(herdr, workspace, target, Some(prompt))
}

fn finish(
    herdr: &Herdr,
    workspace: &WorkspaceView,
    target: &Target,
    prompt: Option<&str>,
) -> AgentOutcome {
    match ensure_inner(herdr, workspace, target, prompt) {
        Ok(outcome) => outcome,
        Err(error) => AgentOutcome::error_for(target, error),
    }
}

/// Where the prompt should go: an agent that already exists, or a new one of a given kind.
#[derive(Debug)]
enum Source<'a> {
    /// Prompt this agent in place.
    Existing(&'a Agent),
    /// Start this kind in a new background tab.
    NewKind(String),
}

/// Apply the Init target policy without touching Herdr.
///
/// `auto_kind` is the first installed CLI, resolved by the caller only for `auto` targets.
fn choose_source<'a>(
    target: &Target,
    agents: &[&'a Agent],
    auto_kind: Option<String>,
) -> Result<Source<'a>> {
    match target {
        Target::Pane(pane) => agents
            .iter()
            .copied()
            .find(|agent| agent.pane_id.as_deref() == Some(pane.as_str()))
            .map(Source::Existing)
            .ok_or_else(|| Error::new(format!("pane {pane} has no agent in this workspace"))),
        Target::Kind(kind) => {
            validate_kind(kind)?;
            Ok(Source::NewKind(kind.clone()))
        }
        Target::Auto => {
            if let Some(agent) = pick_agent(agents) {
                return Ok(Source::Existing(agent));
            }
            match auto_kind {
                Some(kind) => Ok(Source::NewKind(kind)),
                None => Err(Error::new(format!(
                    "no supported agent CLI found on PATH (tried {})",
                    AGENT_ORDER.join(", ")
                ))),
            }
        }
    }
}

fn ensure_inner(
    herdr: &Herdr,
    workspace: &WorkspaceView,
    target: &Target,
    prompt: Option<&str>,
) -> Result<AgentOutcome> {
    let path = workspace.path.as_deref().ok_or_else(|| {
        Error::new(format!(
            "workspace {} has no checkout path; refusing to start or prompt an agent",
            workspace.id
        ))
    })?;
    let prompt = prompt.ok_or_else(|| Error::new("send has no prompt text to submit"))?;

    let all_agents = herdr.agents()?;
    let workspace_agents: Vec<&Agent> = all_agents
        .iter()
        .filter(|agent| agent.workspace_id == workspace.id)
        .collect();

    let auto_kind = match target {
        Target::Auto => detect_kind(),
        Target::Pane(_) | Target::Kind(_) => None,
    };
    let target_request = target.as_request();

    match choose_source(target, &workspace_agents, auto_kind)? {
        Source::Existing(agent) => {
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
            prompt_when_ready(herdr, pane, prompt)?;
            Ok(AgentOutcome {
                state: AgentState::Existing,
                target: Some(target_request),
                kind: agent.agent.clone(),
                name: agent.name.clone(),
                pane: Some(pane.to_string()),
                tab: agent.tab_id.clone(),
                prompted: true,
                reason: None,
            })
        }
        Source::NewKind(kind) => {
            if find_in_path(&kind).is_none() {
                return Err(Error::new(format!(
                    "agent CLI {kind:?} is not installed on PATH"
                )));
            }
            let created = herdr.create_tab(&workspace.id, path)?;
            let name = unique_agent_name(&all_agents, "mindmap")?;
            if let Err(error) = herdr.start_agent(&name, &kind, &created.pane_id) {
                // A first run in a folder asks the agent to trust it; the agent is alive but
                // blocked, so `agent start` reports it as not ready. Accept the default and
                // carry on.
                if !accept_trust(herdr, &created.pane_id)? {
                    return Err(error);
                }
            }
            prompt_when_ready(herdr, &created.pane_id, prompt)?;

            Ok(AgentOutcome {
                state: AgentState::Started,
                target: Some(target_request),
                kind: Some(kind),
                name: Some(name),
                pane: Some(created.pane_id),
                tab: created.tab_id,
                prompted: true,
                reason: None,
            })
        }
    }
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

/// Agent CLIs installed on this host, in `AGENT_ORDER`.
pub fn installed_kinds() -> Vec<String> {
    AGENT_ORDER
        .iter()
        .filter(|kind| find_in_path(kind).is_some())
        .map(|kind| (*kind).to_string())
        .collect()
}

/// Find the first supported agent executable on `PATH`, in `AGENT_ORDER`.
fn detect_kind() -> Option<String> {
    installed_kinds().into_iter().next()
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
            agents_detail: Vec::new(),
            clis: Vec::new(),
            focused: false,
            map: "empty".into(),
            file: None,
            tabs: Some(1),
            panes: Some(1),
        }
    }

    fn agent(pane: &str, kind: &str, status: &str, focused: bool) -> Agent {
        Agent {
            workspace_id: "wX".into(),
            agent: Some(kind.into()),
            name: None,
            pane_id: Some(pane.into()),
            tab_id: Some(format!("{}:t1", pane.split(':').next().unwrap_or("wX"))),
            agent_status: Some(status.into()),
            focused,
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

    #[test]
    fn parses_auto_pane_and_kind_targets() {
        assert_eq!(parse_target(None).ok(), Some(Target::Auto));
        assert_eq!(parse_target(Some("auto")).ok(), Some(Target::Auto));
        assert_eq!(parse_target(Some(" auto ")).ok(), Some(Target::Auto));
        assert_eq!(
            parse_target(Some("pane:wX:p2")).ok(),
            Some(Target::Pane("wX:p2".into()))
        );
        assert_eq!(
            parse_target(Some("kind:claude")).ok(),
            Some(Target::Kind("claude".into()))
        );
    }

    #[test]
    fn rejects_invalid_targets() {
        assert!(parse_target(Some("pane:")).is_err());
        assert!(parse_target(Some("kind:")).is_err());
        assert!(parse_target(Some("kind:not-a-cli")).is_err());
        assert!(parse_target(Some("wX:p2")).is_err());
        assert!(parse_target(Some("workspace:wX")).is_err());
    }

    #[test]
    fn target_request_form_round_trips() {
        for raw in ["auto", "pane:wX:p2", "kind:claude"] {
            let target = parse_target(Some(raw)).unwrap_or(Target::Auto);
            assert_eq!(target.as_request(), raw);
        }
    }

    #[test]
    fn pane_target_picks_that_agent_even_when_another_is_focused() {
        let focused = agent("wX:p1", "codex", "working", true);
        let chosen = agent("wX:p2", "claude", "idle", false);
        let agents = vec![&focused, &chosen];

        let source = choose_source(&Target::Pane("wX:p2".into()), &agents, None)
            .unwrap_or(Source::NewKind("none".into()));
        assert!(matches!(
            source,
            Source::Existing(found) if found.pane_id.as_deref() == Some("wX:p2")
        ));
    }

    #[test]
    fn pane_target_outside_the_workspace_errors() {
        let focused = agent("wX:p1", "codex", "working", true);
        let agents = vec![&focused];
        assert!(choose_source(&Target::Pane("wP:p5".into()), &agents, None).is_err());
        assert!(choose_source(&Target::Pane("wX:p9".into()), &agents, None).is_err());
    }

    #[test]
    fn kind_target_starts_new_even_when_an_agent_exists() {
        let existing = agent("wX:p2", "claude", "idle", true);
        let agents = vec![&existing];

        let source = choose_source(&Target::Kind("claude".into()), &agents, None)
            .unwrap_or(Source::NewKind("none".into()));
        assert!(matches!(source, Source::NewKind(ref kind) if kind == "claude"));
    }

    #[test]
    fn auto_target_reuses_the_focused_agent() {
        let focused = agent("wX:p1", "codex", "working", true);
        let idle = agent("wX:p2", "claude", "idle", false);
        let agents = vec![&focused, &idle];

        let source =
            choose_source(&Target::Auto, &agents, None).unwrap_or(Source::NewKind("none".into()));
        assert!(matches!(
            source,
            Source::Existing(found) if found.pane_id.as_deref() == Some("wX:p1")
        ));
    }

    #[test]
    fn auto_target_avoids_blocked_agents_and_falls_back_to_a_new_cli() {
        let blocked = agent("wX:p1", "codex", "blocked", false);
        let idle = agent("wX:p2", "claude", "idle", false);
        let agents = vec![&blocked, &idle];

        let source =
            choose_source(&Target::Auto, &agents, None).unwrap_or(Source::NewKind("none".into()));
        assert!(matches!(
            source,
            Source::Existing(found) if found.pane_id.as_deref() == Some("wX:p2")
        ));

        let source = choose_source(&Target::Auto, &[], Some("claude".into()))
            .unwrap_or(Source::NewKind("none".into()));
        assert!(matches!(source, Source::NewKind(ref kind) if kind == "claude"));

        assert!(choose_source(&Target::Auto, &[], None).is_err());
    }

    #[test]
    fn target_error_still_names_the_pane_or_kind() {
        let pane = serde_json::to_value(AgentOutcome::error_for(
            &Target::Pane("wX:p2".into()),
            "pane is gone",
        ))
        .unwrap_or_default();
        assert_eq!(
            pane.get("state").and_then(serde_json::Value::as_str),
            Some("error")
        );
        assert_eq!(
            pane.get("target").and_then(serde_json::Value::as_str),
            Some("pane:wX:p2")
        );
        assert_eq!(
            pane.get("pane").and_then(serde_json::Value::as_str),
            Some("wX:p2")
        );

        let kind = serde_json::to_value(AgentOutcome::error_for(
            &Target::Kind("claude".into()),
            "not installed",
        ))
        .unwrap_or_default();
        assert_eq!(
            kind.get("kind").and_then(serde_json::Value::as_str),
            Some("claude")
        );
    }

    #[test]
    fn sync_preflight_rejects_an_unusable_workspace_before_herdr() {
        let herdr = Herdr::new(PathBuf::from("/nonexistent/herdr"));
        let no_path = workspace(None);
        let error = preflight(&herdr, &no_path, &Target::Auto)
            .err()
            .map(|error| error.message().to_string())
            .unwrap_or_default();
        assert!(error.contains("no checkout path"), "{error}");

        let path = workspace(Some("/repo/eisledger"));
        let error = preflight(&herdr, &path, &Target::Kind("not-a-cli".into()))
            .err()
            .map(|error| error.message().to_string())
            .unwrap_or_default();
        assert!(error.contains("unsupported agent kind"), "{error}");
    }
}
