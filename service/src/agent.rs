//! Ensure a workspace has an agent working on its map, and prompt it.
//!
//! Init is allowed to fail in this stage without failing the whole request: the document already
//! exists by the time an agent is touched, so the caller gets the link plus an `agent` object with
//! `state: "error"` and a reason.
//!
//! Policy, ported from the reference and the project README:
//! * agents already in the workspace are prompted, not duplicated;
//! * with no agent, the first installed CLI in a fixed order is started in a new background tab;
//! * a prompt asks the agent to read the afterburn skill and map the workspace checkout.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::herdr::{Agent, CreatedTab, Herdr};
use crate::workspace::WorkspaceView;

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

/// Ensure an agent exists in `workspace` and submit a caller-supplied prompt.
///
/// The caller has already rendered the prompt and validated the target. An existing agent is
/// reused or the chosen kind is started, first-run trust is accepted, and failures are reported
/// in-band in the returned outcome.
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
            let kind = agent.agent.as_deref();
            let status = agent.agent_status.as_deref();
            if is_tui_dsh(kind) {
                if matches!(status, Some("working" | "blocked")) {
                    return Err(Error::new(format!(
                        "dsh in {pane} is {status}; choose the idle dsh pane",
                        status = status.unwrap_or("busy")
                    )));
                }
                herdr.pane_run(pane, &one_line(prompt))?;
            } else {
                prompt_when_ready(herdr, pane, prompt)?;
            }
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
            let created = herdr.create_tab(&workspace.id, path, "afterburn")?;
            let name = unique_agent_name(&all_agents, "afterburn")?;
            if let Err(error) = herdr.start_agent(&name, &kind, &created.pane_id, &[]) {
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

/// What a first-run folder-trust prompt on screen asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trust {
    /// No trust prompt.
    None,
    /// The highlighted choice trusts the folder: Enter.
    Enter,
    /// Claude Code's prompt, highlighting the safe `No, exit` first: Down to `Yes, I trust this
    /// folder`, then Enter (only once the screen shows `Yes` highlighted).
    DownThenEnter,
}

fn trust_prompt(screen: &str) -> Trust {
    if screen.contains("Yes, I trust this folder") {
        return if screen.contains("❯ Yes, I trust this folder") {
            Trust::Enter
        } else {
            Trust::DownThenEnter
        };
    }
    if screen.contains("Trust this folder")
        || screen.contains("enter continue")
        || screen.contains("Trust and continue")
    {
        return Trust::Enter;
    }
    Trust::None
}

/// Accept a first-run folder-trust prompt, if one is on screen.
///
/// Codex (and similar CLIs) block on "Trust this folder?" in a new checkout, with "Trust and
/// continue" highlighted: Enter moves it on. Claude Code highlights "No, exit" first, so the
/// cursor moves to "Yes, I trust this folder" and Enter is pressed only once the screen shows it
/// there. Herdr reports the agent as blocked meanwhile; we wait briefly for the screen to settle.
fn accept_trust(herdr: &Herdr, pane: &str) -> Result<bool> {
    let screen = herdr.read_screen(pane).unwrap_or_default();
    match trust_prompt(&screen) {
        Trust::None => return Ok(false),
        Trust::Enter => {}
        Trust::DownThenEnter => {
            herdr.send_keys(pane, &["down"])?;
            std::thread::sleep(std::time::Duration::from_millis(400));
            let moved = herdr.read_screen(pane).unwrap_or_default();
            if trust_prompt(&moved) != Trust::Enter {
                return Ok(false);
            }
        }
    }
    herdr.send_keys(pane, &["enter"])?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(true)
}

/// The DeepSeek harness TUI: Herdr sees it but its agent prompt protocol rejects it as an
/// unnamed/non-agent pane. Literal pane input is the honest way to reach it.
fn is_tui_dsh(kind: Option<&str>) -> bool {
    matches!(kind, Some("dsh-tui" | "dsh"))
}

/// One line for a terminal TUI; newlines would submit the prompt in pieces.
fn one_line(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// A new agent in place of `target`, for when agents already running lack tools just installed:
/// the same kind as the agent it would have prompted, else the first installed CLI.
pub fn fresh_target(herdr: &Herdr, workspace: &WorkspaceView, target: &Target) -> Result<Target> {
    if let Target::Kind(_) = target {
        return Ok(target.clone());
    }
    let agents = herdr.agents()?;
    let here: Vec<&Agent> = agents
        .iter()
        .filter(|agent| agent.workspace_id == workspace.id)
        .collect();
    let would_prompt = match target {
        Target::Pane(pane) => here
            .iter()
            .copied()
            .find(|agent| agent.pane_id.as_deref() == Some(pane.as_str())),
        _ => pick_agent(&here),
    };
    would_prompt
        .and_then(|agent| agent.agent.clone())
        .filter(|kind| AGENT_ORDER.contains(&kind.as_str()) && find_in_path(kind).is_some())
        .or_else(detect_kind)
        .map(Target::Kind)
        .ok_or_else(|| {
            Error::new(format!(
                "no supported agent CLI found on PATH (tried {})",
                AGENT_ORDER.join(", ")
            ))
        })
}

/// Find the first supported agent executable on `PATH`, in `AGENT_ORDER`.
fn detect_kind() -> Option<String> {
    installed_kinds().into_iter().next()
}

/// A `which`-equivalent over `PATH` without spawning a process.
pub(crate) fn find_in_path(program: &str) -> Option<PathBuf> {
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

/// Herdr's name for an agent the human named `label` (`Agent picker` → `agent-picker`): Herdr
/// names match `[a-z][a-z0-9_-]{0,31}`, so the human's words live on the tab and this slug on
/// the agent. Room is kept for a `-NN` suffix when the name is taken.
pub fn agent_slug(label: &str) -> String {
    let mut slug = String::new();
    for c in label.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.trim_end_matches('-').chars().take(28).collect();
    slug = slug.trim_end_matches('-').to_string();
    if !slug.starts_with(|c: char| c.is_ascii_lowercase()) {
        slug = format!("a-{slug}")
            .trim_end_matches('-')
            .chars()
            .take(28)
            .collect();
    }
    slug
}

/// The CLI arguments for a chosen model (`--model opus`); none for the CLI's own default. A
/// model is a plain token: anything else is refused rather than passed to a shell.
pub fn model_args(model: Option<&str>) -> Result<Vec<String>> {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Ok(Vec::new());
    };
    if model.len() > 64
        || !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
    {
        return Err(Error::new(format!(
            "model {model:?} is not a plain model name"
        )));
    }
    Ok(vec!["--model".to_string(), model.to_string()])
}

/// A new background tab for an agent the human is creating, labelled with its name.
pub fn create_agent_tab(
    herdr: &Herdr,
    workspace: &WorkspaceView,
    label: &str,
) -> Result<CreatedTab> {
    let path = workspace.path.as_deref().ok_or_else(|| {
        Error::new(format!(
            "workspace {} has no checkout path; refusing to start an agent",
            workspace.id
        ))
    })?;
    let label = label.trim();
    let label = if label.is_empty() { "afterburn" } else { label };
    herdr.create_tab(&workspace.id, path, label)
}

/// What starting an agent did: Herdr's name for it, and whether a folder-trust prompt was
/// accepted on the way.
#[derive(Debug, Clone, Serialize)]
pub struct Started {
    pub name: String,
    pub trusted: bool,
}

/// Start `kind` in `pane` without prompting it: the agent the human created in the Agents
/// modal. A first run in a folder asks the CLI to trust it; that prompt is accepted once.
pub fn start_new_agent(
    herdr: &Herdr,
    pane: &str,
    kind: &str,
    label: &str,
    model: Option<&str>,
) -> Result<Started> {
    validate_kind(kind)?;
    if find_in_path(kind).is_none() {
        return Err(Error::new(format!(
            "agent CLI {kind:?} is not installed on PATH"
        )));
    }
    let args = model_args(model)?;
    let agents = herdr.agents()?;
    let name = unique_agent_name(&agents, &agent_slug(label))?;
    match herdr.start_agent(&name, kind, pane, &args) {
        Ok(()) => Ok(Started {
            name,
            trusted: false,
        }),
        Err(error) => {
            if accept_trust(herdr, pane)? {
                Ok(Started {
                    name,
                    trusted: true,
                })
            } else {
                Err(error)
            }
        }
    }
}

/// Wait until the agent in `pane` is idle (or done), accepting a late folder-trust prompt once.
/// Returns the status it settled on; a blocked or missing agent past `timeout` is an error that
/// says what Herdr last reported.
pub fn wait_until_ready(herdr: &Herdr, pane: &str, timeout: std::time::Duration) -> Result<String> {
    let start = std::time::Instant::now();
    let mut trust_tried = false;
    let mut last = String::from("unknown");
    loop {
        let agents = herdr.agents()?;
        let status = agents
            .iter()
            .find(|agent| agent.pane_id.as_deref() == Some(pane))
            .and_then(|agent| agent.agent_status.clone());
        if let Some(status) = status {
            if matches!(status.as_str(), "idle" | "done") {
                return Ok(status);
            }
            if status == "blocked" && !trust_tried {
                trust_tried = true;
                accept_trust(herdr, pane)?;
            }
            last = status;
        }
        if start.elapsed() >= timeout {
            return Err(Error::new(format!(
                "the agent did not become idle within {}s (last status: {last})",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// The last `count` non-empty lines on the agent's screen, to explain why a start failed.
pub fn last_lines(herdr: &Herdr, pane: &str, count: usize) -> Vec<String> {
    let screen = herdr.read_screen(pane).unwrap_or_default();
    let lines: Vec<String> = screen
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    lines[lines.len().saturating_sub(count)..].to_vec()
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
            branch: Some("main".into()),
            agents: 0,
            agent_names: Vec::new(),
            agent_status: "unknown".into(),
            agents_detail: Vec::new(),
            clis: Vec::new(),
            focused: false,
            map: "empty".into(),
            tabs: Some(1),
            panes: Some(1),
            last_sync: None,
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
            terminal_title_stripped: None,
            tab_label: None,
        }
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
    fn claudes_trust_prompt_moves_off_no_before_enter() {
        let claude = " Security guide\n ❯ No, exit\n   Yes, I trust this folder\n Enter to confirm · Esc to cancel";
        assert_eq!(
            trust_prompt(claude),
            Trust::DownThenEnter,
            "Enter here would exit"
        );
        let moved = "   No, exit\n ❯ Yes, I trust this folder";
        assert_eq!(trust_prompt(moved), Trust::Enter);
        assert_eq!(
            trust_prompt("Trust this folder? › Trust and continue"),
            Trust::Enter,
            "codex"
        );
        assert_eq!(trust_prompt("> ready"), Trust::None);
    }

    #[test]
    fn a_human_name_becomes_a_herdr_agent_name() {
        assert_eq!(agent_slug("Agent picker"), "agent-picker");
        assert_eq!(agent_slug("  UI review (v2)! "), "ui-review-v2");
        assert_eq!(agent_slug("3D canvas"), "a-3d-canvas");
        assert_eq!(agent_slug("Été"), "t");
        assert_eq!(agent_slug(""), "a");
        assert!(agent_slug(&"x".repeat(80)).len() <= 28);
    }

    #[test]
    fn a_model_is_a_plain_cli_argument() {
        assert_eq!(
            model_args(Some("opus")).unwrap(),
            vec!["--model".to_string(), "opus".to_string()]
        );
        assert!(model_args(None).unwrap().is_empty());
        assert!(model_args(Some("  ")).unwrap().is_empty());
        assert!(model_args(Some("opus; rm -rf /")).is_err());
    }

    #[test]
    fn unique_names_skip_live_ones() {
        let agents = vec![Agent {
            workspace_id: "wP".into(),
            agent: Some("codex".into()),
            name: Some("afterburn".into()),
            pane_id: Some("wP:p1".into()),
            tab_id: Some("wP:t1".into()),
            agent_status: Some("idle".into()),
            focused: false,
            terminal_title_stripped: None,
            tab_label: None,
        }];
        assert_eq!(
            unique_agent_name(&agents, "afterburn").unwrap_or_default(),
            "afterburn-2"
        );
        assert_eq!(
            unique_agent_name(&[], "afterburn").unwrap_or_default(),
            "afterburn"
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
