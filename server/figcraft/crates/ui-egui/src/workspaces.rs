//! The Workspaces screen: every Herdr workspace on the project host, with or without a map.
//!
//! Background workers share one versioned node connection per machine. Opening a workspace
//! returns its session id, worktree path and branch; the engine joins through NodeLiveLink.
//! A successful welcome opens the document with its existing page tree.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{OnceLock, RwLock};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::Deserialize;

use crate::FigApp;
use crate::a11y;
use crate::agent_menu;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// Where the node listens unless `MINDMAP_NODE` or `--node` says otherwise.
const DEFAULT_SERVICE: &str = "127.0.0.1:47822";

/// The list's width (the detail gets the rest of the window).
const SIDEBAR: f32 = 300.0;
/// One workspace row in the browser's sidebar.
const ROW_H: f32 = 54.0;
/// A machine row, and the quiet "+ New workspace" / "+ Add machine" rows.
#[cfg(not(target_arch = "wasm32"))]
const MACHINE_H: f32 = 30.0;
const PLUS_H: f32 = 26.0;
/// How far machine rows and the plus rows sit in from the sidebar's edges.
const INSET: f32 = 8.0;

/// How long to wait for a connection to the node.
/// How long the workspace list may take to answer.
#[cfg(not(target_arch = "wasm32"))]
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// Open and Init publish a document and may start an agent: they take much longer.
#[cfg(not(target_arch = "wasm32"))]
const ACTION_TIMEOUT: Duration = Duration::from_secs(120);
/// The address given with `--service`, when it was.
static SERVICE: OnceLock<RwLock<String>> = OnceLock::new();

/// Use `address` for this run (`--node <address>` in the desktop app).
pub fn set_service(address: &str) {
    let address = address.trim();
    if !address.is_empty() {
        *SERVICE.get_or_init(|| RwLock::new(address.to_string())).write().unwrap_or_else(|p| p.into_inner()) = address.to_string();
    }
}

/// The node's address: `--node`, then `MINDMAP_NODE`, then the default.
pub fn service_addr() -> String {
    if let Some(address) = SERVICE.get() {
        return address.read().unwrap_or_else(|p| p.into_inner()).clone();
    }
    std::env::var("MINDMAP_NODE")
        .ok()
        .map(|address| address.trim().to_string())
        .filter(|address| !address.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVICE.to_string())
}

/// One Herdr workspace, as the node lists it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    /// A linked worktree rather than the repository's own checkout.
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub agents: u32,
    /// The CLIs running in the workspace (`codex`, `dsh`, `claude`, `opencode`), one each.
    #[serde(default)]
    pub agent_names: Vec<String>,
    /// Every agent in the workspace, with the pane Init can address.
    #[serde(default)]
    pub agents_detail: Vec<AgentDetail>,
    #[serde(default)]
    pub agent_status: String,
    /// The workspace Herdr has focused.
    #[serde(default)]
    pub focused: bool,
    /// `"map"`, `"empty"` or `"none"`.
    #[serde(default)]
    pub map: String,
    /// The checked-out branch, when the path is a git checkout.
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tabs: Option<u32>,
    #[serde(default)]
    pub panes: Option<u32>,
    #[serde(default)]
    pub last_sync: Option<LastSync>,
}

/// One agent in a workspace, as Init may address it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentDetail {
    /// The Herdr pane the agent lives in (`wX:p2`); Init addresses it as `pane:<pane>`.
    #[serde(default)]
    pub pane: String,
    /// The agent CLI kind (`codex`, `claude`, `kimi`, …).
    #[serde(default)]
    pub kind: String,
    /// Herdr's display name for the agent, when it has one.
    #[serde(default)]
    pub name: Option<String>,
    /// The name of the pane's tab, when someone named it: what Herdr calls the pane.
    #[serde(default)]
    pub tab: Option<String>,
    /// Its lifecycle state (`idle`, `working`, `blocked`, `done`, …).
    #[serde(default)]
    pub status: String,
    /// Focused in the user's Herdr UI.
    #[serde(default)]
    pub focused: bool,
    /// The pane's title: what the agent says it is doing.
    #[serde(default)]
    pub title: Option<String>,
}

/// The newest commit that synced the map with the code.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LastSync {
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub subject: String,
    /// Committer date, ISO 8601.
    #[serde(default)]
    pub at: String,
}

/// What Init should do: ask an agent already in the workspace, or start a new CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An agent already running in the workspace, addressed by its pane.
    Existing { pane: String, kind: String, status: String },
    /// A new agent of this kind.
    New { kind: String },
}

impl Target {
    /// The `target` string the service reads (`pane:wX:p2` or `kind:claude`).
    pub fn line(&self) -> String {
        match self {
            Target::Existing { pane, .. } => format!("pane:{pane}"),
            Target::New { kind } => format!("kind:{kind}"),
        }
    }

    /// The agent kind this target uses.
    pub fn kind(&self) -> &str {
        match self {
            Target::Existing { kind, .. } | Target::New { kind } => kind,
        }
    }
}

/// What the workspace has: a live map, an empty document, or no document yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapState {
    /// A map has been drawn and published.
    Map,
    /// A document exists but the map is still empty.
    Empty,
    /// The workspace has no document yet.
    #[default]
    None,
}

impl MapState {
    pub fn of(state: &str) -> MapState {
        match state {
            "map" => MapState::Map,
            "empty" => MapState::Empty,
            _ => MapState::None,
        }
    }

    /// The row badge.
    fn badge(self) -> &'static str {
        match self {
            MapState::Map => "MAP",
            MapState::Empty => "EMPTY",
            MapState::None => "INIT",
        }
    }

    /// The one action the detail offers.
    fn action(self) -> &'static str {
        match self {
            MapState::Map => "Open map",
            MapState::Empty | MapState::None => "Init map",
        }
    }
}

impl Workspace {
    pub fn map_state(&self) -> MapState {
        MapState::of(&self.map)
    }

    pub(crate) fn name(&self) -> &str {
        if self.label.trim().is_empty() { &self.id } else { &self.label }
    }
}

/// The target a freshly selected workspace starts on: an agent that can be picked (not busy),
/// Herdr's focused one first; with none, the first installed CLI; with neither, none (the service
/// chooses).
fn default_target(w: &Workspace, clis: &[String]) -> Option<Target> {
    let free = |agent: &&AgentDetail| !agent_busy(&agent.status);
    let agent = w.agents_detail.iter().filter(free).find(|agent| agent.focused).or_else(|| w.agents_detail.iter().find(free));
    if let Some(agent) = agent {
        return Some(Target::Existing { pane: agent.pane.clone(), kind: agent.kind.clone(), status: agent.status.clone() });
    }
    clis.iter().map(|kind| kind.trim()).find(|kind| !kind.is_empty()).map(|kind| Target::New { kind: kind.to_string() })
}

/// An agent in the middle of something (or waiting on its human) can't take Init; the agent menu
/// shows it disabled.
fn agent_busy(status: &str) -> bool {
    crate::agent_menu::is_busy(status)
}

/// An agent kind as shown: the service may not know it.
fn display_kind(kind: &str) -> &str {
    let kind = kind.trim();
    if kind.is_empty() { "agent" } else { kind }
}

/// What a request from the screen is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The workspace list.
    List,
    /// Open or Init.
    Act,
}

/// A request the service answers.
#[derive(Clone, Debug)]
enum Request {
    List,
    Act { workspace: String, init: bool, target: Option<Target> },
}

impl Request {
    fn kind(&self) -> Kind {
        match self {
            Request::List => Kind::List,
            Request::Act { .. } => Kind::Act,
        }
    }

    /// The JSON line the service reads.
    fn line(&self) -> String {
        match self {
            Request::List => r#"{"id":1,"method":"workspaces"}"#.to_string(),
            Request::Act { workspace, init, target } => {
                let method = if *init { "init" } else { "open" };
                let mut request = serde_json::json!({"id": 2, "method": method, "workspace": workspace});
                // Only Init takes a target, and only when the human (or Retry) chose one.
                if *init && let Some(target) = target {
                    request["target"] = serde_json::Value::String(target.line());
                }
                request.to_string()
            }
        }
    }

    fn busy(&self) -> Busy {
        match self {
            Request::List => Busy::List,
            Request::Act { workspace, init, target } => Busy::Open { workspace: workspace.clone(), init: *init, target: target.clone() },
        }
    }

    /// How long its answer may take.
    #[cfg(not(target_arch = "wasm32"))]
    fn timeout(&self) -> Duration {
        match self {
            Request::List => LIST_TIMEOUT,
            Request::Act { .. } => ACTION_TIMEOUT,
        }
    }
}

/// What is in flight: the whole list, or one workspace's Open / Init.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Busy {
    List,
    Open { workspace: String, init: bool, target: Option<Target> },
}

impl Busy {
    fn for_workspace(&self, id: &str) -> bool {
        matches!(self, Busy::Open { workspace, .. } if workspace == id)
    }
}

/// What an error's Retry asks for again.
#[derive(Clone, Debug)]
enum Retry {
    List,
    Action {
        workspace: String,
        init: bool,
        /// The same target the failed request used.
        target: Option<Target>,
    },
    /// A join that failed; the link is still good until the service says otherwise.
    Join(String),
}

/// A reply from the service. Unknown fields are ignored and missing ones take their defaults, so
/// a service that grows a field never breaks the screen.
#[derive(Debug, Default, Deserialize)]
struct ServiceReply {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    workspaces: Option<Vec<Workspace>>,
    /// The agent CLIs installed on the project host, in the service's order.
    #[serde(default)]
    clis: Vec<String>,
    /// What Init installs first (`skill for claude`, `MCP for codex`); empty when ready, and from
    /// a node that doesn't say.
    #[serde(default)]
    agent_tools_missing: Vec<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    branch: String,
    /// The node's version, with the workspace list.
    #[serde(default)]
    node: Option<String>,
}

/// The Workspaces screen's state.
#[derive(Default)]
pub struct State {
    /// The workspaces last listed.
    pub rows: Vec<Workspace>,
    #[cfg(not(target_arch = "wasm32"))]
    pub hosts: crate::node_hosts::State,
    /// Routing metadata retained while the engine receives its first welcome.
    pending_route: Option<String>,
    pending_live: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    pending_reopen: Option<crate::node::SessionRef>,
    events_refresh: bool,
    events_at: f64,
    /// The selected workspace id (its detail shows on the right).
    pub selected: Option<String>,
    /// The machines unfolded in the sidebar, by address; several can be open at once.
    #[cfg(not(target_arch = "wasm32"))]
    pub open: std::collections::BTreeSet<String>,
    /// The machine in use opens on the first draw; after that the human decides.
    #[cfg(not(target_arch = "wasm32"))]
    open_seeded: bool,
    /// The workspaces of open machines other than the one in use, by address.
    #[cfg(not(target_arch = "wasm32"))]
    pub lists: BTreeMap<String, HostList>,
    /// The last error, shown in muted red with a Retry.
    pub error: Option<String>,
    /// The request in flight, if any.
    pub busy: Option<Busy>,
    /// The agent CLIs installed on the project host, from the last list.
    pub clis: Vec<String>,
    /// What Init installs before it starts an agent, from the last list.
    pub agent_tools_missing: Vec<String>,
    /// The listing node's version (`0.1.0`).
    pub node_version: Option<String>,
    /// What Init should use for the selected workspace; `None` lets the service choose.
    pub target: Option<Target>,
    /// Init's agent menu (the composer's button and menu) is open.
    pub agent_menu_open: bool,
    /// Init's agent menu: its search and arrow cursor.
    agent_menu: crate::agent_menu::MenuState,
    /// The channel a request's reply arrives on.
    rx: Option<Receiver<(Kind, Result<ServiceReply, String>)>>,
    /// The link the screen joins now (then shows the map).
    pub join: Option<String>,
    /// The workspace whose link is in [`join`](Self::join): the composer's active workspace once
    /// the join succeeds.
    pub join_workspace: Option<String>,
    /// What Retry asks for again.
    retry: Option<Retry>,
    /// The list was asked for once, when the screen first showed.
    started: bool,
    /// The workspace pane is open beside the map. It survives a switch between workspaces.
    pub pane_open: bool,
    /// A repository with several worktrees shows its worktree list; this shows the selected
    /// worktree's own detail (its Init) instead, after Open on a worktree without a map.
    pub worktree_init: bool,
}

/// One open machine's workspaces, listed beside the machine in use.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct HostList {
    pub rows: Vec<Workspace>,
    pub clis: Vec<String>,
    pub agent_tools_missing: Vec<String>,
    pub node: Option<String>,
    pub error: Option<String>,
    /// A listing has answered at least once.
    pub loaded: bool,
    rx: Option<Receiver<Result<ServiceReply, String>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HostList {
    pub fn loading(&self) -> bool {
        self.rx.is_some()
    }
}

impl State {
    /// List `address`'s workspaces on a thread of its own, without leaving the machine in use.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn list_host(&mut self, ctx: &egui::Context, address: &str) {
        let list = self.lists.entry(address.to_string()).or_default();
        if list.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        list.rx = Some(rx);
        let (address, context) = (address.to_string(), ctx.clone());
        let spawned = std::thread::Builder::new().name("figcraft-machine-list".into()).spawn(move || {
            let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list_reply(&address)))
                .unwrap_or_else(|_| Err("The workspace request failed unexpectedly.".to_string()));
            let _ = tx.send(reply);
            context.request_repaint();
        });
        if let Err(e) = spawned {
            list.rx = None;
            list.error = Some(format!("Could not start the workspace request: {e}"));
        }
    }

    /// Take in the other machines' finished listings.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_lists(&mut self) {
        for list in self.lists.values_mut() {
            let received = match list.rx.as_ref().map(Receiver::try_recv) {
                None | Some(Err(mpsc::TryRecvError::Empty)) => continue,
                Some(Ok(reply)) => reply,
                Some(Err(mpsc::TryRecvError::Disconnected)) => Err("The workspace request ended without an answer.".to_string()),
            };
            list.rx = None;
            list.loaded = true;
            match received {
                Ok(reply) => {
                    list.rows = reply.workspaces.unwrap_or_default();
                    list.clis = reply.clis;
                    list.agent_tools_missing = reply.agent_tools_missing;
                    list.node = reply.node.filter(|v| !v.trim().is_empty());
                    list.error = None;
                }
                Err(e) => list.error = Some(e),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn queue_branch_reopen(&mut self, selected: &str, address: &str, event: &serde_json::Value, active: Option<&crate::node::SessionRef>) {
        if selected != address || event["metadata"]["active"].as_bool() != Some(false) {
            return;
        }
        if let Some(node) = active.filter(|node| node.address == address && event["session"].as_str() == Some(node.session.as_str())) {
            self.pending_reopen = Some(node.clone());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn take_branch_reopen(&mut self, selected: &str, active: Option<&crate::node::SessionRef>) -> Option<crate::node::SessionRef> {
        if self.busy.is_some() || self.pending_live.is_some() || self.join.is_some() {
            return None;
        }
        self.pending_reopen.take().filter(|queued| {
            queued.address == selected && active.is_some_and(|node| node.address == queued.address && node.session == queued.session)
        })
    }

    /// Ask for the workspace list, on a thread of its own.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() {
            return;
        }
        self.error = None;
        self.retry = Some(Retry::List);
        self.start(ctx, Request::List);
    }

    /// Select `workspace` and put the Init target back on its first agent (or first CLI).
    pub fn select(&mut self, workspace: &str) {
        self.worktree_init = false;
        self.selected = Some(workspace.to_string());
        self.error = None;
        self.agent_menu_open = false;
        self.target = self.rows.iter().find(|w| w.id == workspace).and_then(|w| default_target(w, &self.clis));
    }

    /// Open `workspace`'s map, or Init it, on a thread of its own.
    pub fn act(&mut self, ctx: &egui::Context, workspace: &str, init: bool) {
        if self.busy.is_some() {
            return;
        }
        // A row's action may name a workspace that isn't selected yet: select it first, so the
        // target belongs to the workspace Init is about.
        if self.selected.as_deref() != Some(workspace) {
            self.select(workspace);
        }
        let target = if init { self.target.clone() } else { None };
        self.act_with(ctx, workspace, init, target);
    }

    /// Run Open / Init with `target`; Retry comes through here with the failed request's target.
    fn act_with(&mut self, ctx: &egui::Context, workspace: &str, init: bool, target: Option<Target>) {
        if self.busy.is_some() {
            return;
        }
        if self.selected.as_deref() != Some(workspace) {
            self.select(workspace);
        }
        self.selected = Some(workspace.to_string());
        if init {
            self.target = target.clone();
        }
        self.error = None;
        // Open and Init both publish the workspace's map; remember which one for the join.
        self.join_workspace = Some(workspace.to_string());
        self.retry = Some(Retry::Action { workspace: workspace.to_string(), init, target: target.clone() });
        self.start(ctx, Request::Act { workspace: workspace.to_string(), init, target });
    }

    /// Stop waiting for an Open: its reply is dropped and the current map stays. A join the
    /// engine already started is left to finish in the background.
    pub fn cancel_open(&mut self) {
        if matches!(self.busy, Some(Busy::Open { .. })) {
            self.rx = None;
            self.busy = None;
        }
        self.join = None;
        self.join_workspace = None;
        self.pending_live = None;
        self.pending_route = None;
        self.error = None;
        self.retry = None;
    }

    /// Do again what failed.
    pub fn retry(&mut self, ctx: &egui::Context) {
        match self.retry.clone() {
            Some(Retry::Action { workspace, init, target }) => self.act_with(ctx, &workspace, init, target),
            Some(Retry::Join(link)) => {
                self.join = Some(link);
                ctx.request_repaint();
            }
            Some(Retry::List) | None => self.refresh(ctx),
        }
    }

    /// The picked agent turned busy: `Pick another agent` goes back to the workspace's default
    /// and opens the agent menu on it.
    fn pick_another(&mut self, w: &Workspace) {
        self.target = default_target(w, &self.clis);
        self.open_agent_menu();
    }

    /// Open Init's agent menu on an empty search.
    pub(crate) fn open_agent_menu(&mut self) {
        self.agent_menu_open = true;
        self.agent_menu.opened();
    }

    /// Keep the chosen target when a new list still offers it; refresh its status; otherwise fall
    /// back to the workspace's first agent or CLI.
    fn reconcile_target(&mut self) {
        let Some(id) = self.selected.as_deref() else {
            self.target = None;
            return;
        };
        let Some(w) = self.rows.iter().find(|w| w.id == id) else {
            self.selected = None;
            self.target = None;
            return;
        };
        let refreshed = match &self.target {
            Some(Target::Existing { pane, .. }) => w.agents_detail.iter().find(|agent| &agent.pane == pane).map(|agent| Target::Existing {
                pane: agent.pane.clone(),
                kind: agent.kind.clone(),
                status: agent.status.clone(),
            }),
            Some(Target::New { kind }) => self.clis.iter().any(|cli| cli.trim() == kind.trim()).then(|| Target::New { kind: kind.clone() }),
            None => None,
        };
        self.target = refreshed.or_else(|| default_target(w, &self.clis));
    }

    /// Start `request` on a named thread; its reply wakes the window.
    fn start(&mut self, ctx: &egui::Context, request: Request) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.busy = Some(request.busy());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let address = service_addr();
            let context = ctx.clone();
            let kind = request.kind();
            let spawned = std::thread::Builder::new().name("figcraft-workspaces".into()).spawn(move || {
                let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exchange(&address, &request)))
                    .unwrap_or_else(|_| Err("The workspace request failed unexpectedly.".to_string()));
                let _ = tx.send((kind, reply));
                context.request_repaint();
            });
            if let Err(e) = spawned {
                self.rx = None;
                self.busy = None;
                self.error = Some(format!("Could not start the workspace request: {e}"));
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ctx, tx);
            self.rx = None;
            self.busy = None;
            self.error = Some("The node is only available in the desktop app.".to_string());
        }
    }

    /// Take in a finished request's reply: the rows, the join link or the error.
    fn poll(&mut self) {
        let received = self.rx.as_ref().map(Receiver::try_recv);
        let received = match received {
            None => return,
            Some(Ok(reply)) => reply,
            Some(Err(mpsc::TryRecvError::Empty)) => return,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.rx = None;
                self.busy = None;
                self.error = Some("The workspace request ended without an answer.".to_string());
                return;
            }
        };
        self.rx = None;
        let init = matches!(self.busy, Some(Busy::Open { init: true, .. }));
        self.busy = None;
        let (kind, result) = received;
        match kind {
            Kind::List => match result {
                Ok(reply) if reply.ok => {
                    self.rows = reply.workspaces.unwrap_or_default();
                    self.clis = reply.clis;
                    self.agent_tools_missing = reply.agent_tools_missing;
                    self.node_version = reply.node.filter(|v| !v.trim().is_empty());
                    if self.selected.is_none() {
                        self.target = None;
                        // The detail always shows a workspace: Herdr's focused one, else the first.
                        if let Some(id) = self.rows.iter().find(|w| w.focused).or_else(|| self.rows.first()).map(|w| w.id.clone()) {
                            self.select(&id);
                        }
                    } else {
                        self.reconcile_target();
                    }
                    self.error = None;
                    self.retry = None;
                }
                Ok(reply) => self.error = Some(reply.error.unwrap_or_else(|| "The node refused the request.".to_string())),
                Err(e) => self.error = Some(e),
            },
            Kind::Act => match result {
                Ok(reply) if reply.ok => match reply.session.filter(|session| !session.trim().is_empty()) {
                    Some(session) => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.join = Some(
                                crate::node::SessionRef {
                                    address: service_addr(),
                                    session,
                                    workspace: self.join_workspace.clone().unwrap_or_default(),
                                    path: reply.path,
                                    branch: reply.branch,
                                }
                                .route(),
                            );
                        }
                        #[cfg(target_arch = "wasm32")]
                        let _ = session;
                        if init {
                            // Init installed the agent tools before it started the agent.
                            self.agent_tools_missing.clear();
                        }
                        self.error = None;
                        self.retry = None;
                    }
                    None => self.error = Some("The node did not return a session id.".to_string()),
                },
                Ok(reply) => self.error = Some(reply.error.unwrap_or_else(|| "The node refused the request.".to_string())),
                Err(e) => self.error = Some(e),
            },
        }
    }
}

/// Read one newline-terminated JSON reply from the service; anything but a well-formed reply is
/// an error the screen shows, never a panic.
#[cfg(not(target_arch = "wasm32"))]
fn exchange(address: &str, request: &Request) -> Result<ServiceReply, String> {
    let value =
        crate::node::rpc(address, serde_json::from_str(&request.line()).map_err(|e| format!("Invalid node request: {e}"))?, request.timeout())?;
    serde_json::from_value(value).map_err(|e| format!("The node sent an answer this app can't read: {e}"))
}

/// One machine's workspace list through its pooled connection; a node that needs an update says so.
#[cfg(not(target_arch = "wasm32"))]
fn list_reply(address: &str) -> Result<ServiceReply, String> {
    if let Some(reason) = crate::node::update_required(address)? {
        return Err(reason);
    }
    let value = crate::node::rpc(address, serde_json::json!({"id": 1, "method": "workspaces"}), LIST_TIMEOUT)?;
    let reply: ServiceReply = serde_json::from_value(value).map_err(|e| format!("The node sent an answer this app can't read: {e}"))?;
    if reply.ok { Ok(reply) } else { Err(reply.error.unwrap_or_else(|| "The node refused the request.".to_string())) }
}

/// A compact monitor-ish palette over the app's tokens, from the colour theme
/// ([`crate::map_colors::get`]). Classic keeps the map design's paper, inks and rules, and Classic
/// Dark the tokens' surfaces with lightened inks, exactly as before the themes.
pub(crate) struct Skin {
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) border: Color32,
    pub(crate) text: Color32,
    pub(crate) muted: Color32,
    pub(crate) faint: Color32,
    pub(crate) accent: Color32,
    pub(crate) accent_hover: Color32,
    pub(crate) selected: Color32,
    pub(crate) soft: Color32,
    pub(crate) map_ink: Color32,
    pub(crate) danger: Color32,
    pub(crate) danger_bg: Color32,
    /// The dark Open map button and its label.
    pub(crate) primary: Color32,
    pub(crate) primary_hover: Color32,
    pub(crate) on_primary: Color32,
}

impl Skin {
    pub(crate) fn of(t: &Tokens) -> Skin {
        let theme = crate::map_colors::theme();
        if !matches!(theme, crate::map_colors::Theme::Classic | crate::map_colors::Theme::ClassicDark) {
            let c = theme.colors();
            // The dark Open map button: the theme's ink, its hover a step towards the paper.
            let primary_hover = if theme.dark() { Color32::WHITE } else { mix(c.ink, c.panel, 0.15) };
            return Skin {
                bg: c.canvas,
                panel: c.panel,
                border: c.border,
                text: c.ink,
                muted: c.muted,
                faint: c.faint,
                accent: c.accent,
                accent_hover: c.accent_hover,
                selected: c.selected,
                soft: c.soft,
                map_ink: c.map_ink,
                danger: c.danger,
                danger_bg: with_alpha(c.danger, if theme.dark() { 28 } else { 16 }),
                primary: c.ink,
                primary_hover,
                on_primary: c.panel,
            };
        }
        if t.dark {
            let danger = Color32::from_rgb(0xE8, 0x9A, 0x8A);
            Skin {
                bg: t.canvas,
                panel: t.panel,
                border: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                accent: Color32::from_rgb(0xD9, 0x7A, 0x5F),
                accent_hover: Color32::from_rgb(0xE8, 0x8E, 0x75),
                selected: Color32::from_rgb(0x3A, 0x30, 0x2C),
                soft: t.hover,
                map_ink: Color32::from_rgb(0x7F, 0xBD, 0xB5),
                danger,
                danger_bg: with_alpha(danger, 28),
                primary: Color32::from_rgb(0xEC, 0xEA, 0xE6),
                primary_hover: Color32::WHITE,
                on_primary: Color32::from_rgb(0x1A, 0x1A, 0x1A),
            }
        } else {
            // The map design's paper, inks and rules.
            let danger = Color32::from_rgb(0x9E, 0x3B, 0x2C);
            Skin {
                bg: Color32::from_rgb(0xF4, 0xF4, 0xF2),
                panel: t.panel,
                border: Color32::from_rgb(0xE3, 0xE2, 0xDF),
                text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                muted: Color32::from_rgb(0x6E, 0x6C, 0x68),
                faint: Color32::from_rgb(0x9A, 0x96, 0x8F),
                accent: Color32::from_rgb(0xB5, 0x45, 0x2F),
                accent_hover: Color32::from_rgb(0xC2, 0x55, 0x3E),
                selected: Color32::from_rgb(0xF1, 0xF0, 0xED),
                soft: Color32::from_rgb(0xF7, 0xF6, 0xF3),
                map_ink: Color32::from_rgb(0x2F, 0x6F, 0x6A),
                danger,
                danger_bg: with_alpha(danger, 16),
                primary: Color32::from_rgb(0x1A, 0x1A, 0x1A),
                primary_hover: Color32::from_rgb(0x33, 0x33, 0x33),
                on_primary: Color32::WHITE,
            }
        }
    }
}

/// Draw the Workspaces screen in the window's `ui`.
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Take in what the request threads found before drawing the rows they change.
    app.ws.poll();
    // A service reply with a link: join it as the human would have, then show the map.
    if join_ready(app) {
        return;
    }
    // The app's keyboard navigation reserves Tab for the canvas; this screen has no canvas, so
    // hand Tab back to egui: it moves through the rows, Refresh, the split button and the menu.
    let step = ui.input(|i| {
        i.key_pressed(egui::Key::Tab).then_some(if i.modifiers.shift { egui::FocusDirection::Previous } else { egui::FocusDirection::Next })
    });
    if let Some(direction) = step {
        ctx.memory_mut(|memory| memory.move_focus(direction));
    }
    // The first time the screen shows, list the workspaces; Refresh asks again.
    if !app.ws.started {
        app.ws.started = true;
        #[cfg(not(target_arch = "wasm32"))]
        crate::node_hosts::select(&mut app.ws.hosts, crate::node_hosts::resolve(&service_addr()));
        #[cfg(target_arch = "wasm32")]
        app.ws.refresh(&ctx);
    }
    let skin = Skin::of(&Tokens::get(&ctx));
    // On macOS the window buttons sit in a toolbar band at the top left; the screen starts below.
    let band = if app.ui.mac_inset() { crate::MAC_BAND } else { 0.0 };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(skin.bg)).show(ui, |ui| {
        let rect = ui.max_rect();
        if band > 0.0 {
            crate::titlebar_drag(ui, Rect::from_min_size(rect.min, vec2(rect.width(), band)));
        }
        let body = rect;
        // The list keeps its width; the detail takes the rest, but never less than 320 px.
        let side = SIDEBAR.min((body.width() - 320.0).max(240.0));
        let list = Rect::from_min_size(body.min, vec2(side, body.height()));
        let detail_rect = Rect::from_min_max(pos2(list.max.x, body.min.y), body.max);
        sidebar(app, ui, &skin, list, band);
        detail(app, ui, &skin, detail_rect, band);
        ui.painter().line_segment([pos2(list.max.x, body.min.y), pos2(list.max.x, body.max.y)], Stroke::new(1.0, skin.border));
    });
    // The New workspace modal first, so the Add machine modal it opens draws above it.
    crate::new_workspace::show(app, ui);
    crate::machine_modal::show(app, ui);
}

/// The map's sidebar, laid out as Herdr's: it runs the window's full height. At the top, a row as
/// tall as the header with the window buttons (macOS) and the toggle that hides it; then the page
/// tree (design: `Sync pane redesign`). The workspace is the window tab's name and ⌃⌘O's list, its
/// agent count is in the status bar, and Settings and Add machine are in the Actions palette.
pub fn pane(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    app.ws.poll();
    if join_ready(app) {
        return;
    }
    if !app.ws.started {
        app.ws.started = true;
        #[cfg(not(target_arch = "wasm32"))]
        crate::node_hosts::select(&mut app.ws.hosts, crate::node_hosts::resolve(&service_addr()));
        #[cfg(target_arch = "wasm32")]
        app.ws.refresh(&ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(host) = crate::node_hosts::poll(&mut app.ws.hosts) {
        use_host(app, &ctx, &host);
    }
    #[cfg(not(target_arch = "wasm32"))]
    app.ws.poll_lists();
    let skin = Skin::of(&Tokens::get(&ctx));
    let s = &skin;
    let area = ui.max_rect();
    ui.painter().rect_filled(area, CornerRadius::ZERO, s.panel);
    // Its inner edge.
    ui.painter().vline(area.max.x - 0.5, area.y_range(), Stroke::new(1.0, s.border));

    // The top row: the window drags from it; the window buttons sit at its left on macOS.
    let inset = app.ui.mac_inset();
    let top = Rect::from_min_size(area.min, vec2(area.width(), if inset { crate::MAC_BAND } else { 44.0 }));
    if app.ui.title_bar != crate::TitleBar::Native {
        crate::titlebar_drag(ui, top);
    }
    let toggle = Rect::from_min_max(pos2(top.max.x - 38.0, top.min.y), pos2(top.max.x - 10.0, top.max.y));
    let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(toggle).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if crate::sidebar_toggle(&mut cell, true).clicked() {
        app.ws.pane_open = false;
    }

    // The page tree, right under the top row: the window tab names the workspace (⌃⌘O switches
    // it), and Settings and Add machine live in the Actions palette (⇧⌘P) and the menus.
    let tree = Rect::from_min_max(pos2(area.min.x, top.max.y), area.max);
    crate::page_tree::docked(app, ui, tree);
}

/// The creation modals over a map (New workspace, ⇧⌘N, and the Add machine card it opens). The
/// status bar draws them, since it shows whether or not the sidebar does; the Workspaces screen
/// draws its own.
pub fn map_modals(app: &mut FigApp, ui: &mut Ui) {
    crate::new_workspace::show(app, ui);
    crate::machine_modal::show(app, ui);
}

/// A section's name over its rows (Pages).
pub(crate) const SECTION_H: f32 = 31.0;

/// A section's name, small and quiet, over its rows.
pub(crate) fn section(ui: &Ui, s: &Skin, rect: Rect, label: &str) {
    ui.painter().text(pos2(rect.min.x + 16.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.5), s.faint);
}

/// A small plus.
pub(crate) fn plus(ui: &Ui, c: Pos2, ink: Color32) {
    let stroke = Stroke::new(1.3, ink);
    ui.painter().line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], stroke);
    ui.painter().line_segment([pos2(c.x, c.y - 5.0), pos2(c.x, c.y + 5.0)], stroke);
}

/// A workspace's agent count, as the status bar writes it after the branch.
pub(crate) fn pane_agents(w: &Workspace) -> String {
    match w.agents {
        0 => "no agent".to_string(),
        1 => "1 agent".to_string(),
        n => format!("{n} agents"),
    }
}

/// Start the node join and wait for its document welcome before showing the map.
fn join_ready(app: &mut FigApp) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(id) = app.ws.pending_live {
        let live = app.session.live.get(id);
        match live.map(|live| &live.status) {
            Some(figcraft_engine::live::Status::Live) if live.is_some_and(|live| live.doc_uid.is_some()) => {
                app.ws.pending_live = None;
                app.ws.pending_route = None;
                app.ws.busy = None;
                app.ws.error = None;
                app.ui.home = false;
                update_active(app);
                app.machine_modal.close();
                app.new_workspace.close();
                return true;
            }
            Some(figcraft_engine::live::Status::Failed { reason }) => {
                app.ws.error = Some(format!("The map could not be joined: {reason}"));
                app.ws.pending_live = None;
                app.ws.busy = None;
                app.ws.retry = app.ws.pending_route.take().map(Retry::Join);
            }
            _ => return false,
        }
    }
    let Some(route) = app.ws.join.take() else { return false };
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Opening the same node session again selects its existing document; a connection has
        // one peer per session, so it must not join twice from two local tabs.
        let existing = app.session.docs.iter().position(|doc| {
            let live = app.session.live_of(doc.uid);
            matches!(
                live.status,
                figcraft_engine::live::Status::Live | figcraft_engine::live::Status::Reconnecting { .. } | figcraft_engine::live::Status::Connecting
            ) && live.info.as_ref().is_some_and(|info| info.link == route)
        });
        if let Some(index) = existing {
            app.session.active = index;
            if app.session.live().status != figcraft_engine::live::Status::Live
                && let Ok(target) = crate::node::SessionRef::parse(&route)
            {
                crate::node::retry_session(&target);
            }
            app.ui.home = false;
            update_active(app);
            app.machine_modal.close();
            app.new_workspace.close();
            return true;
        }
    }
    if let Some(reply) = app.run("collab.join", serde_json::json!({"link": route.clone()})) {
        app.ws.pending_live = reply.get("session").and_then(serde_json::Value::as_u64);
        app.ws.pending_route = Some(route);
        if app.ws.pending_live.is_none() {
            app.ws.error = Some("The engine did not start the node session.".into());
        } else {
            app.ws.busy = Some(Busy::Open { workspace: app.ws.join_workspace.clone().unwrap_or_default(), init: false, target: None });
        }
        return false;
    }
    app.ws.error = Some(app.ui.last_error.take().unwrap_or_else(|| "The map could not be joined.".into()));
    app.ws.retry = Some(Retry::Join(route));
    false
}

/// Resolve requests from the document in front, never from the host picker.
#[cfg(not(target_arch = "wasm32"))]
pub fn active_node(app: &FigApp) -> Result<crate::node::SessionRef, String> {
    let live = app.session.live();
    if live.status != figcraft_engine::live::Status::Live {
        return Err("The node is offline. Click the connection status to retry; your comments are kept.".into());
    }
    node_identity(app).ok_or_else(|| "Open a node workspace first.".into())
}

/// Workspace identity survives a temporary outage; only requests require a live connection.
#[cfg(not(target_arch = "wasm32"))]
pub fn node_identity(app: &FigApp) -> Option<crate::node::SessionRef> {
    crate::node::SessionRef::parse(&app.session.live().info.as_ref()?.link).ok()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn reconnect(app: &mut FigApp) {
    if let Some(target) = node_identity(app)
        && crate::node::retry_session(&target)
    {
        app.toast("Reconnecting to the node. Your comments are kept; send them when connected.");
    } else {
        crate::switcher::toggle(app);
        app.toast("Choose the workspace to reconnect. Your comments are kept.");
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn update_active(app: &mut FigApp) {
    app.active_workspace = node_identity(app).map(|target| target.workspace);
}

/// Live job and Herdr notifications use the same socket as the document.
#[cfg(not(target_arch = "wasm32"))]
pub fn poll_node_events(app: &mut FigApp, ctx: &egui::Context) {
    // Checkout transitions must also finish while the workspace sidebar is hidden.
    app.ws.poll();
    join_ready(app);
    let selected = service_addr();
    let active = active_node(app).ok().or_else(|| app.ws.pending_route.as_deref().and_then(|route| crate::node::SessionRef::parse(route).ok()));
    for (address, event) in crate::node::take_events() {
        match event["event"].as_str() {
            Some("herdr") => {
                if address == selected {
                    app.ws.events_refresh = true;
                } else if app.ws.open.contains(&address) && app.ws.lists.contains_key(&address) {
                    // Another open machine changed: list it again.
                    app.ws.list_host(ctx, &address);
                }
                if active.as_ref().is_some_and(|node| node.address == address) {
                    app.compose.targets_at = -10.0;
                }
            }
            Some("job") => crate::windows::sync_event(app, ctx, &address, event),
            Some("session") => {
                app.ws.queue_branch_reopen(&selected, &address, &event, active.as_ref());
                if active.as_ref().is_some_and(|node| node.address == address && event["session"].as_str() == Some(node.session.as_str()))
                    && let Some(error) = event["error"].as_str()
                {
                    app.toast(format!("Node session: {error}"));
                }
                if address == selected {
                    app.ws.events_refresh = true;
                }
            }
            _ => {}
        }
    }
    if let Some(node) = app.ws.take_branch_reopen(&selected, active.as_ref()) {
        app.ws.act(ctx, &node.workspace, false);
    }
    let now = ctx.input(|input| input.time);
    if app.ws.events_refresh && app.ws.busy.is_none() {
        if now - app.ws.events_at >= 0.5 {
            app.ws.events_refresh = false;
            app.ws.events_at = now;
            app.ws.refresh(ctx);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Use `host` on the Workspaces screen: drop the old host's answers and list its workspaces.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_host(app: &mut FigApp, ctx: &egui::Context, host: &crate::node_hosts::Host) {
    stash_rows(app);
    app.ws.open.insert(host.address.clone());
    app.ws.lists.remove(&host.address);
    set_service(&host.address);
    // Discard old-host responses before requesting the new host's workspaces.
    app.ws.rx = None;
    app.ws.busy = None;
    app.ws.selected = None;
    app.ws.rows.clear();
    app.ws.clis.clear();
    app.ws.agent_tools_missing.clear();
    app.ws.target = None;
    app.ws.join = None;
    app.ws.join_workspace = None;
    app.ws.hosts.selected = Some(host.clone());
    app.ws.refresh(ctx);
}

/// Bring `workspace` to the front in that machine's Herdr, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_in_herdr(address: &str, workspace: &str) {
    let (address, request) = (address.to_string(), serde_json::json!({"method": "workspace.focus", "workspace": workspace}));
    let _ = std::thread::Builder::new().name("figcraft-herdr-focus".into()).spawn(move || {
        let _ = crate::node::rpc(&address, request, LIST_TIMEOUT);
    });
}

/// The sidebar row the machine in use is drawn under: its SSH address, or this computer's.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn in_use_key(app: &FigApp) -> Option<String> {
    match app.ws.hosts.selected.as_ref() {
        Some(host) if host.ssh.is_some() => Some(host.address.clone()),
        _ => crate::node_hosts::hosts(&app.ws.hosts).first().map(|host| host.address.clone()),
    }
}

/// Keep the machine in use's workspaces listed under its row while another machine takes over.
#[cfg(not(target_arch = "wasm32"))]
fn stash_rows(app: &mut FigApp) {
    if let Some(key) = in_use_key(app) {
        let list = HostList {
            rows: std::mem::take(&mut app.ws.rows),
            clis: std::mem::take(&mut app.ws.clis),
            agent_tools_missing: std::mem::take(&mut app.ws.agent_tools_missing),
            node: app.ws.node_version.take(),
            loaded: true,
            ..HostList::default()
        };
        app.ws.lists.insert(key, list);
    }
}

/// Make `host` the machine in use with the workspaces already listed for it, then select `id`.
/// Its node answered that listing, so no setup runs; Open and Init now go to it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn switch_to(app: &mut FigApp, host: &crate::node_hosts::Host, id: Option<&str>) {
    if app.ws.busy.is_some() {
        return;
    }
    let Some(list) = app.ws.lists.remove(&host.address) else { return };
    stash_rows(app);
    set_service(&host.address);
    app.ws.rx = None;
    app.ws.hosts.selected = Some(host.clone());
    app.ws.rows = list.rows;
    app.ws.clis = list.clis;
    app.ws.agent_tools_missing = list.agent_tools_missing;
    app.ws.node_version = list.node;
    app.ws.error = None;
    app.ws.selected = None;
    app.ws.target = None;
    if let Some(id) = id {
        app.ws.select(id);
    }
}

/// The repository a workspace belongs to; linked worktrees share their base repo's key.
fn repo_key(w: &Workspace) -> String {
    w.repo.clone().filter(|repo| !repo.trim().is_empty()).or_else(|| w.path.clone()).unwrap_or_else(|| w.id.clone())
}

/// One row per repository: the base checkout and its linked worktrees as branches.
pub(crate) fn grouped(rows: &[Workspace]) -> Vec<(Workspace, Vec<Workspace>)> {
    let mut buckets: BTreeMap<String, Vec<&Workspace>> = BTreeMap::new();
    for w in rows {
        buckets.entry(repo_key(w)).or_default().push(w);
    }
    let mut groups = Vec::new();
    for (_, mut members) in buckets {
        members.sort_by_key(|w| w.worktree);
        let base = members.first().map(|w| (*w).clone()).unwrap_or_default();
        let branches = members.into_iter().cloned().collect();
        groups.push((base, branches));
    }
    groups
}

/// The branch a workspace represents: git's branch name, else the worktree's short name.
pub(crate) fn branch_label(w: &Workspace) -> String {
    w.branch
        .clone()
        .filter(|branch| !branch.trim().is_empty())
        .or_else(|| w.path.as_deref().and_then(|path| Path::new(path).file_name()).map(|name| name.to_string_lossy().to_string()))
        .unwrap_or_else(|| w.name().to_string())
}

/// A quiet "+   label" row inset under the list: New workspace and Add machine.
fn plus_row(ui: &mut Ui, s: &Skin, id: Id, label: &str) -> bool {
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), PLUS_H), Sense::hover());
    plus_row_at(ui, s, id, outer, label)
}

fn plus_row_at(ui: &mut Ui, s: &Skin, id: Id, outer: Rect, label: &str) -> bool {
    let rect = Rect::from_min_max(pos2(outer.min.x + INSET, outer.min.y), pos2(outer.max.x - INSET, outer.max.y));
    let resp = ui.interact(rect, id, Sense::click());
    a11y::button(&resp, label);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), s.soft);
    }
    focus_ring(ui, s, rect, &resp, 6);
    let ink = if resp.hovered() { s.text } else { s.muted };
    ui.painter().text(pos2(rect.min.x + 8.0, rect.min.y + 5.0), Align2::LEFT_TOP, format!("+   {label}"), regular(10.5), ink);
    resp.clicked()
}

/// The machines and their workspaces, with Add machine pinned under them.
fn sidebar(app: &mut FigApp, ui: &mut Ui, s: &Skin, rect: Rect, band: f32) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, s.panel);
    // Below the window buttons' band when there is one.
    let top = rect.min.y + band.max(16.0);
    ui.painter().text(pos2(rect.min.x + 16.0, top), Align2::LEFT_TOP, "WORKSPACES", mono(9.0), s.muted);

    // Add machine sits at the foot, under a divider, whatever the list's length.
    let add = Rect::from_min_size(pos2(rect.min.x, rect.max.y - 16.0 - PLUS_H), vec2(rect.width(), PLUS_H));
    let rule = add.min.y - 8.0;
    let list = Rect::from_min_max(pos2(rect.min.x, top + 20.0), pos2(rect.max.x, rule - 8.0));
    let ctx = ui.ctx().clone();
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        ui.set_clip_rect(list.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("workspaces_list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            #[cfg(not(target_arch = "wasm32"))]
            machine_list(app, ui, s, &ctx);
            #[cfg(target_arch = "wasm32")]
            flat_list(app, ui, s, &ctx);
        });
    });
    ui.painter().line_segment([pos2(rect.min.x + INSET, rule), pos2(rect.max.x - INSET, rule)], Stroke::new(1.0, s.border));
    if plus_row_at(ui, s, Id::new("workspaces_add_machine"), add, "Add machine") {
        crate::machine_modal::open(app);
    }
}

/// The machines, each a toggle over its own workspaces. Any number can be open at once: the one
/// in use lists through the screen's state, the others through their own listing. Choosing a
/// workspace under another machine makes that machine the one in use.
#[cfg(not(target_arch = "wasm32"))]
fn machine_list(app: &mut FigApp, ui: &mut Ui, s: &Skin, ctx: &egui::Context) {
    let ready = crate::node_hosts::poll(&mut app.ws.hosts);
    app.ws.poll_lists();
    // This computer once, first; then the SSH machines.
    let hosts: Vec<_> = crate::node_hosts::hosts(&app.ws.hosts)
        .iter()
        .enumerate()
        .filter(|(i, host)| *i == 0 || host.ssh.is_some())
        .map(|(_, host)| host.clone())
        .collect();
    let in_use = in_use_key(app);
    if !app.ws.open_seeded {
        app.ws.open_seeded = true;
        app.ws.open.extend(in_use.clone());
    }
    let preparing = crate::node_hosts::preparing(&app.ws.hosts);
    let mut toggled: Option<(crate::node_hosts::Host, bool)> = None;
    let mut setup: Option<crate::node_hosts::Host> = None;
    let mut switch: Option<(crate::node_hosts::Host, Option<String>, Option<Chosen>)> = None;
    for (index, host) in hosts.iter().enumerate() {
        if index > 0 {
            ui.add_space(16.0);
        }
        let is_in_use = in_use.as_deref() == Some(host.address.as_str());
        let expanded = app.ws.open.contains(&host.address);
        let state = if is_in_use {
            machine_state(host, true, preparing, app.ws.rows.len(), app.ws.node_version.as_deref())
        } else {
            match app.ws.lists.get(&host.address) {
                Some(list) if list.loading() => machine_state(host, true, true, 0, None),
                Some(list) if list.loaded && list.error.is_none() => machine_state(host, true, false, list.rows.len(), list.node.as_deref()),
                _ => machine_state(host, false, false, 0, None),
            }
        };
        if machine_row(ui, s, host, &state, expanded) {
            toggled = Some((host.clone(), is_in_use));
        }
        ui.add_space(8.0);
        if !expanded {
            if !is_in_use && !app.ws.lists.get(&host.address).is_some_and(|list| list.loaded) {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
                ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y), Align2::LEFT_TOP, "Connect to list its workspaces", mono(8.5), s.faint);
            }
            continue;
        }
        if is_in_use {
            let rows = app.ws.rows.clone();
            let mut chosen = None;
            for (base, branches) in &grouped(&rows) {
                let selected = app.ws.selected.as_deref().is_some_and(|id| branches.iter().any(|b| b.id == id));
                if let Some(c) = row(ui, s, base, selected) {
                    chosen = Some(c);
                }
            }
            match chosen {
                Some(Chosen::Select(id)) => app.ws.select(&id),
                Some(Chosen::Act { workspace, init }) => app.ws.act(ctx, &workspace, init),
                None => {}
            }
            let (error, busy, started) = (app.ws.error.clone(), app.ws.busy.is_some(), app.ws.started);
            if app.ws.rows.is_empty() {
                empty_list(app, ui, s, ctx, error.as_deref(), busy, started);
            }
            ui.add_space(4.0);
            if plus_row(ui, s, Id::new(("workspaces_new", &host.address)), "New workspace") {
                crate::new_workspace::open(app);
            }
            continue;
        }
        // Another open machine: its own listing, or why there is none.
        let (rows, error, loading) = match app.ws.lists.get(&host.address) {
            Some(list) => (list.rows.clone(), list.error.clone(), list.loading()),
            None => (Vec::new(), None, true),
        };
        for (base, _) in &grouped(&rows) {
            if let Some(chosen) = row(ui, s, base, false) {
                let id = match &chosen {
                    Chosen::Select(id) | Chosen::Act { workspace: id, .. } => id.clone(),
                };
                switch = Some((host.clone(), Some(id), Some(chosen)));
            }
        }
        if let Some(error) = error {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 2, bottom: 6 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.add(egui::Label::new(egui::RichText::new(error).font(mono(8.5)).color(s.danger)).wrap());
                if link(ui, s, Id::new(("workspaces_setup", &host.address)), "Set up this machine…", s.muted) {
                    setup = Some(host.clone());
                }
            });
        } else if loading && rows.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
            ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y), Align2::LEFT_TOP, "Listing its workspaces…", mono(8.5), s.faint);
        } else if rows.is_empty() {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
            ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y), Align2::LEFT_TOP, "No workspaces yet", mono(8.5), s.faint);
        }
        ui.add_space(4.0);
        if plus_row(ui, s, Id::new(("workspaces_new", &host.address)), "New workspace") && !loading {
            switch = Some((host.clone(), None, None));
            crate::new_workspace::open(app);
        }
    }
    host_notice(ui, s, &mut app.ws.hosts);
    if let Some((host, is_in_use)) = toggled
        && !app.ws.open.remove(&host.address)
    {
        app.ws.open.insert(host.address.clone());
        if !is_in_use {
            app.ws.list_host(ctx, &host.address);
        }
    }
    if let Some((host, id, chosen)) = switch {
        switch_to(app, &host, id.as_deref());
        if let Some(Chosen::Act { workspace, init }) = chosen
            && in_use_key(app).as_deref() == Some(host.address.as_str())
        {
            app.ws.act(ctx, &workspace, init);
        }
    }
    if let Some(host) = ready {
        use_host(app, ui.ctx(), &host);
    }
    if let Some(host) = setup {
        crate::node_hosts::activate(&mut app.ws.hosts, &host);
    }
}

/// What a host setup asks for (Install Herdr, Update node) or reports, set in the sidebar's
/// type as an inset card so long reasons wrap inside the list instead of running across it.
#[cfg(not(target_arch = "wasm32"))]
fn host_notice(ui: &mut Ui, s: &Skin, hosts: &mut crate::node_hosts::State) {
    if !crate::node_hosts::has_notice(hosts) {
        return;
    }
    ui.add_space(12.0);
    egui::Frame::NONE
        .outer_margin(egui::Margin::symmetric(INSET as i8, 0))
        .inner_margin(egui::Margin::symmetric(10, 9))
        .fill(s.selected)
        .corner_radius(CornerRadius::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            let style = ui.style_mut();
            style.override_font_id = Some(regular(10.5));
            style.visuals.override_text_color = Some(s.muted);
            style.wrap_mode = Some(egui::TextWrapMode::Wrap);
            crate::node_hosts::status(ui, hosts);
        });
}

/// The machine's second line: where it is, and what the app knows of it.
#[cfg(not(target_arch = "wasm32"))]
fn machine_state(host: &crate::node_hosts::Host, selected: bool, preparing: bool, workspaces: usize, node: Option<&str>) -> String {
    let count = format!("{workspaces} workspace{}", if workspaces == 1 { "" } else { "s" });
    match (&host.ssh, selected) {
        (None, true) => format!("local · {count}"),
        (None, false) => "local".to_string(),
        (Some(_), true) if preparing => "ssh · connecting…".to_string(),
        (Some(_), true) => match node {
            Some(version) => format!("ssh · connected · node {version}"),
            None => "ssh · connected".to_string(),
        },
        (Some(target), false) => format!("ssh · {target}"),
    }
}

/// The browser build has no machines: one flat workspace list.
#[cfg(target_arch = "wasm32")]
fn flat_list(app: &mut FigApp, ui: &mut Ui, s: &Skin, ctx: &egui::Context) {
    let (error, busy, started) = (app.ws.error.clone(), app.ws.busy.is_some(), app.ws.started);
    if app.ws.rows.is_empty() {
        empty_list(app, ui, s, ctx, error.as_deref(), busy, started);
    } else {
        let rows = app.ws.rows.clone();
        let groups = grouped(&rows);
        let mut chosen = None;
        for (base, branches) in &groups {
            let selected = app.ws.selected.as_deref().is_some_and(|id| branches.iter().any(|b| b.id == id));
            if let Some(c) = row(ui, s, base, selected) {
                chosen = Some(c);
            }
        }
        match chosen {
            Some(Chosen::Select(id)) => app.ws.select(&id),
            Some(Chosen::Act { workspace, init }) => app.ws.act(ctx, &workspace, init),
            None => {}
        }
    }
    ui.add_space(4.0);
    if plus_row(ui, s, Id::new("workspaces_new"), "New workspace") {
        crate::new_workspace::open(app);
    }
}

/// One machine: its name and state, and the arrow that folds its workspaces away.
#[cfg(not(target_arch = "wasm32"))]
fn machine_row(ui: &mut Ui, s: &Skin, host: &crate::node_hosts::Host, state: &str, expanded: bool) -> bool {
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), MACHINE_H), Sense::hover());
    let rect = Rect::from_min_max(pos2(outer.min.x + INSET, outer.min.y), pos2(outer.max.x - INSET, outer.max.y));
    let resp = ui.interact(rect, Id::new(("workspaces_machine", &host.address)), Sense::click());
    a11y::button(&resp, &format!("{} · {}", host.name, if expanded { "open" } else { "folded" }));
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), s.soft);
    }
    focus_ring(ui, s, rect, &resp, 6);
    let room = rect.width() - 50.0;
    let name = widgets::elide_font(ui, &host.name, room, semibold(12.0));
    ui.painter().text(pos2(rect.min.x + 10.0, rect.min.y + 2.0), Align2::LEFT_TOP, name, semibold(12.0), s.text);
    let state = widgets::elide_font(ui, state, room, mono(8.5));
    ui.painter().text(pos2(rect.min.x + 10.0, rect.min.y + 18.0), Align2::LEFT_TOP, state, mono(8.5), s.faint);
    // A plain arrow, drawn: up when open, down when folded.
    let c = pos2(rect.max.x - 11.0, rect.min.y + 13.0);
    let ink = if resp.hovered() { s.muted } else { s.faint };
    let points = if expanded {
        vec![pos2(c.x - 3.5, c.y + 2.0), pos2(c.x + 3.5, c.y + 2.0), pos2(c.x, c.y - 2.5)]
    } else {
        vec![pos2(c.x - 3.5, c.y - 2.0), pos2(c.x + 3.5, c.y - 2.0), pos2(c.x, c.y + 2.5)]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, ink, Stroke::NONE));
    resp.clicked()
}

/// A row asks for one thing: select its workspace, or open / initialise it.
enum Chosen {
    Select(String),
    Act { workspace: String, init: bool },
}

/// MAP and EMPTY in the map ink, INIT in the accent.
pub(crate) fn badge_tint(s: &Skin, kind: MapState) -> Color32 {
    match kind {
        MapState::Map | MapState::Empty => s.map_ink,
        MapState::None => s.accent,
    }
}

/// One workspace: name, path and agents on the left, its map state on the right. A click selects
/// it; a double click opens or initialises it.
fn row(ui: &mut Ui, s: &Skin, w: &Workspace, selected: bool) -> Option<Chosen> {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let kind = w.map_state();
    a11y::button(&resp, &format!("{} · {} · {}", w.name(), kind.badge(), agents_line(w)));
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.soft);
    }
    focus_ring(ui, s, rect.shrink(1.0), &resp, 0);

    let badge = kind.badge();
    let badge_w = text_width(ui, badge, mono(8.5));
    ui.painter().text(pos2(rect.max.x - 16.0, rect.min.y + 7.0), Align2::RIGHT_TOP, badge, mono(8.5), badge_tint(s, kind));

    let name = w.name();
    let room = (rect.width() - 32.0 - badge_w - 12.0).max(40.0);
    let name_shown = widgets::elide_font(ui, name, room, semibold(12.5));
    ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y + 6.0), Align2::LEFT_TOP, &name_shown, semibold(12.5), s.text);
    let path = w.path.as_deref().or(w.repo.as_deref()).unwrap_or("no folder");
    let path_shown = widgets::elide_start_font(ui, path, (rect.width() - 32.0).max(40.0), mono(8.5));
    ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y + 25.0), Align2::LEFT_TOP, &path_shown, mono(8.5), s.muted);
    let agents = agents_line(w);
    ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y + 39.0), Align2::LEFT_TOP, &agents, mono(8.5), s.faint);

    let resp = if name_shown != name || path_shown != path { resp.on_hover_text(format!("{name}\n{path}")) } else { resp };
    if resp.double_clicked() {
        Some(Chosen::Act { workspace: w.id.clone(), init: kind != MapState::Map })
    } else if resp.clicked() {
        Some(Chosen::Select(w.id.clone()))
    } else {
        None
    }
}

fn empty_list(app: &mut FigApp, ui: &mut Ui, s: &Skin, ctx: &egui::Context, error: Option<&str>, busy: bool, started: bool) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(16, 18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        if let Some(error) = error {
            ui.label(egui::RichText::new("Can't read the workspaces").font(semibold(12.5)).color(s.text));
            paragraph(ui, error, 11.5, s.danger);
            ui.add_space(6.0);
            if secondary_button_alloc(ui, s, Id::new("workspaces_list_retry"), "Retry", !busy) {
                app.ws.retry(ctx);
            }
        } else if busy || !started {
            paragraph(ui, "Connecting to the node…", 12.0, s.muted);
        } else {
            ui.label(egui::RichText::new("No workspaces yet").font(semibold(12.5)).color(s.text));
            paragraph(ui, "Create one in Herdr on the project host, then Refresh.", 11.5, s.muted);
            ui.add_space(6.0);
            if secondary_button_alloc(ui, s, Id::new("workspaces_empty_refresh"), "Refresh", !busy) {
                app.ws.refresh(ctx);
            }
        }
    });
}

fn detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, rect: Rect, band: f32) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, s.bg);
    let selected = app.ws.selected.as_deref().and_then(|id| app.ws.rows.iter().find(|w| w.id == id)).cloned();
    // The detail keeps the design's 16 pt lead over the sidebar label.
    let inner = Rect::from_min_max(rect.min + vec2(32.0, band.max(16.0) + 16.0), rect.max - vec2(32.0, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt("workspaces_detail").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(inner.width().min(760.0));
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            let worktrees = selected.as_ref().map(|w| worktrees_of(&app.ws.rows, w)).unwrap_or_default();
            match &selected {
                Some(w) if worktrees.len() > 1 && !app.ws.worktree_init => project_detail(app, ui, s, w, &worktrees),
                Some(w) => workspace_detail(app, ui, s, w),
                None => intro(app, ui, s),
            }
            ui.add_space(16.0);
        });
    });
}

fn intro(app: &mut FigApp, ui: &mut Ui, s: &Skin) {
    eyebrow(ui, s, "WORKSPACES");
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Herdr workspaces are the projects.").font(semibold(26.0)).color(s.text));
    ui.add_space(10.0);
    paragraph(
        ui,
        "A workspace can have a live map, an empty document, or no document at all. Select one on the left to open its map or Init the first agent.",
        12.5,
        s.muted,
    );
    if let Some(error) = app.ws.error.clone() {
        ui.add_space(22.0);
        if error_card(ui, s, &error, app.ws.busy.is_some()) {
            app.ws.retry(ui.ctx());
        }
    }
    let summary = match (app.ws.busy.is_some(), app.ws.rows.len()) {
        (true, 0) => "Reading the workspaces from the node…",
        (_, 0) if app.ws.started => "No workspaces listed yet.",
        _ => "",
    };
    if !summary.is_empty() {
        ui.add_space(20.0);
        ui.label(egui::RichText::new(summary).font(mono(9.0)).color(s.faint));
    }
}

fn workspace_detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, w: &Workspace) {
    let kind = w.map_state();
    // Reached from a repository's worktree list: the way back to it.
    if app.ws.worktree_init && worktrees_of(&app.ws.rows, w).len() > 1 {
        if link(ui, s, Id::new("workspaces_all_worktrees"), "←  All worktrees", s.muted) {
            app.ws.worktree_init = false;
        }
        ui.add_space(10.0);
    }
    if kind != MapState::Map {
        init_detail(app, ui, s, w);
        return;
    }
    let ctx = ui.ctx().clone();
    eyebrow(ui, s, "SELECTED WORKSPACE");
    ui.add_space(6.0);
    ui.label(egui::RichText::new(w.name()).font(semibold(26.0)).color(s.text));
    ui.add_space(10.0);
    ui.add(egui::Label::new(egui::RichText::new(meta_line(w)).font(mono(11.0)).color(s.muted)).truncate());
    ui.add_space(9.0);
    ui.scope(|ui| {
        ui.set_max_width(600.0);
        paragraph(ui, explanation(kind), 12.5, s.muted);
    });

    // Open map and the agents beside it.
    ui.add_space(32.0);
    let busy = app.ws.busy.clone();
    let busy_here = busy.as_ref().is_some_and(|b| b.for_workspace(&w.id));
    let label = action_label(kind, w, busy.as_ref());
    let mut run = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if primary_button(ui, s, Id::new(("workspace_action", &w.id)), &label, busy.is_none()) {
            run = true;
        }
        ghost_button(ui, s, Id::new(("workspace_agents", &w.id)), &agents_line(w), true, false);
    });
    if run {
        app.ws.act(&ctx, &w.id, false);
    }
    if busy_here {
        let line = match &busy {
            Some(Busy::Open { init: true, target: Some(Target::New { kind }), .. }) => {
                format!("The node is starting {}; the overview appears as the agent draws it.", display_kind(kind))
            }
            Some(Busy::Open { init: true, target: Some(Target::Existing { kind, .. }), .. }) => {
                format!("The node is asking {}; the overview appears as the agent draws it.", display_kind(kind))
            }
            Some(Busy::Open { init: true, .. }) => "The node is starting the agent; the overview appears as the agent draws it.".to_string(),
            Some(Busy::Open { init: false, .. }) => "Opening the live design…".to_string(),
            _ => String::new(),
        };
        if !line.is_empty() {
            ui.add_space(10.0);
            paragraph(ui, &line, 11.0, s.faint);
        }
    }
    if let Some(error) = app.ws.error.clone() {
        ui.add_space(16.0);
        if error_card(ui, s, &error, app.ws.busy.is_some()) {
            app.ws.retry(&ctx);
        }
    }

    ui.add_space(24.0);
    divider(ui, s);
    ui.add_space(17.0);
    machine_block(app, ui, s);
    ui.add_space(14.0);
    divider(ui, s);
    ui.add_space(15.0);
    agents_block(ui, s, w);
    #[cfg(not(target_arch = "wasm32"))]
    if !w.agents_detail.is_empty() {
        ui.add_space(6.0);
        if link(ui, s, Id::new(("workspace_open_herdr", &w.id)), "Open in Herdr  →", s.map_ink) {
            open_in_herdr(&service_addr(), &w.id);
        }
    }
    ui.add_space(16.0);
    divider(ui, s);
    ui.add_space(15.0);
    sync_block(ui, s, w);
    ui.add_space(26.0);
    hint(ui, s, "No map yet? The row says INIT — open it and press Init map.");
}

/// The worktrees of `w`'s repository: the main checkout first, then the linked ones by branch.
pub(crate) fn worktrees_of(rows: &[Workspace], w: &Workspace) -> Vec<Workspace> {
    let key = repo_key(w);
    let mut all: Vec<Workspace> = rows.iter().filter(|row| repo_key(row) == key).cloned().collect();
    all.sort_by(|a, b| a.worktree.cmp(&b.worktree).then_with(|| branch_label(a).cmp(&branch_label(b))));
    all
}

/// What the selected worktree's line says about its map.
fn map_line(kind: MapState) -> &'static str {
    match kind {
        MapState::Map => "map ready",
        MapState::Empty => "empty map",
        MapState::None => "no map yet",
    }
}

/// A repository with several worktrees (the Branches design): choose a worktree, then open it.
/// Every branch is a folder of its own; choosing one never checks out a branch anywhere.
fn project_detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, w: &Workspace, worktrees: &[Workspace]) {
    let ctx = ui.ctx().clone();
    let repo = w.repo.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| worktrees.first().map(|b| b.name().to_string()).unwrap_or_default());
    #[cfg(not(target_arch = "wasm32"))]
    let machine = app.ws.hosts.selected.as_ref().map(|host| host.name.clone()).unwrap_or_else(|| "This computer".to_string());
    #[cfg(target_arch = "wasm32")]
    let machine = "This computer".to_string();
    eyebrow(ui, s, "SELECTED PROJECT");
    ui.add_space(8.0);
    ui.label(egui::RichText::new(&repo).font(semibold(26.0)).color(s.text));
    ui.add_space(14.0);
    ui.label(egui::RichText::new(format!("{machine}  ·  {} worktrees", worktrees.len())).font(mono(11.0)).color(s.muted));
    ui.add_space(21.0);
    paragraph(ui, "Choose a worktree. Its map, agents and Burns stay with that folder.", 12.5, s.muted);
    ui.add_space(34.0);
    eyebrow(ui, s, "OPEN A WORKTREE");
    ui.add_space(12.0);
    let width = ui.available_width().min(600.0);
    let mut chosen = None;
    for (i, branch) in worktrees.iter().enumerate() {
        if i > 0 {
            ui.add_space(8.0);
        }
        if worktree_card(ui, s, branch, width, branch.id == w.id) {
            chosen = Some(branch.id.clone());
        }
    }
    if let Some(id) = chosen {
        app.ws.select(&id);
    }

    let kind = w.map_state();
    let branch = branch_label(w);
    ui.add_space(16.0);
    let hint = format!("Selected: {branch}  ·  {}  ·  {}", agents_line(w).replace(" · ", " "), map_line(kind));
    ui.label(egui::RichText::new(hint).font(regular(10.0)).color(s.map_ink));
    ui.add_space(9.0);
    let busy = app.ws.busy.clone();
    let label = match &busy {
        Some(Busy::Open { workspace, .. }) if workspace == &w.id => "Opening…".to_string(),
        _ => format!("Open {branch}  →"),
    };
    if accent_button(ui, s, Id::new(("worktree_open", &w.id)), &label, busy.is_none()) {
        if kind == MapState::Map {
            app.ws.act(&ctx, &w.id, false);
        } else {
            // A worktree without a map opens its Init; no agent starts until asked.
            app.ws.worktree_init = true;
        }
    }
    if let Some(error) = app.ws.error.clone() {
        ui.add_space(16.0);
        if error_card(ui, s, &error, app.ws.busy.is_some()) {
            app.ws.retry(&ctx);
        }
    }
    ui.add_space(16.0);
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Opens the map in the selected worktree. A worktree without a map opens Init; choosing it never starts an agent automatically.",
                )
                .font(mono(11.0))
                .color(s.muted),
            )
            .wrap(),
        );
    });
}

/// One worktree in the project detail: branch, path, and its map state; the chosen one in the
/// accent with a check.
fn worktree_card(ui: &mut Ui, s: &Skin, w: &Workspace, width: f32, chosen: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 58.0), Sense::click());
    let kind = w.map_state();
    a11y::button(&resp, &format!("{} · {}", branch_label(w), kind.badge()));
    let (fill, rule) = if chosen {
        (with_alpha(s.accent, if s.panel == Color32::WHITE { 18 } else { 40 }), s.accent)
    } else if resp.hovered() {
        (s.soft, s.border)
    } else {
        (s.panel, s.border)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, rule), StrokeKind::Inside);
    focus_ring(ui, s, rect, &resp, 5);
    let state = match kind {
        MapState::Map => "MAP",
        MapState::Empty => "EMPTY",
        MapState::None => "NO MAP",
    };
    let tint = if chosen {
        s.accent
    } else if kind == MapState::Map {
        s.map_ink
    } else {
        s.muted
    };
    let state_x = rect.max.x - 84.0;
    // The monospace face has no check glyph: the mark is drawn.
    if chosen {
        check_mark(ui, pos2(state_x - 12.0, rect.min.y + 15.5), tint);
    }
    ui.painter().text(pos2(state_x, rect.min.y + 10.0), Align2::LEFT_TOP, state, mono(9.0), tint);
    let room = (state_x - 12.0 - (rect.min.x + 14.0)).max(40.0);
    let name = widgets::elide_font(ui, &branch_label(w), room, regular(12.0));
    ui.painter().text(pos2(rect.min.x + 14.0, rect.min.y + 8.0), Align2::LEFT_TOP, name, regular(12.0), s.text);
    let path = w.path.as_deref().unwrap_or("no folder");
    let path = widgets::elide_start_font(ui, path, rect.width() - 28.0, mono(9.0));
    ui.painter().text(pos2(rect.min.x + 14.0, rect.min.y + 32.0), Align2::LEFT_TOP, path, mono(9.0), s.muted);
    resp.clicked()
}

/// A small drawn check mark centred on `c`.
pub(crate) fn check_mark(ui: &Ui, c: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    ui.painter().line_segment([pos2(c.x - 3.5, c.y), pos2(c.x - 1.0, c.y + 2.5)], stroke);
    ui.painter().line_segment([pos2(c.x - 1.0, c.y + 2.5), pos2(c.x + 3.5, c.y - 2.5)], stroke);
}

/// The accent call to action of the worktree list: 34 pt, label 12 pt from the left.
fn accent_button(ui: &mut Ui, s: &Skin, id: Id, label: &str, enabled: bool) -> bool {
    let width = (text_width(ui, label, semibold(11.0)) + 24.0).max(236.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let fill = if !enabled {
        s.soft
    } else if resp.hovered() {
        Color32::from_rgb(0xC2, 0x55, 0x3E)
    } else {
        s.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().text(
        pos2(rect.min.x + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        semibold(11.0),
        if enabled { Color32::WHITE } else { s.faint },
    );
    focus_ring(ui, s, rect, &resp, 6);
    resp.clicked() && enabled
}

/// The chosen agent turned busy after it was picked: the second step names what it is doing.
fn turned_busy<'a>(w: &'a Workspace, target: Option<&Target>) -> Option<&'a AgentDetail> {
    match target {
        Some(Target::Existing { pane, .. }) => w.agents_detail.iter().find(|agent| &agent.pane == pane && agent_busy(&agent.status)),
        _ => None,
    }
}

/// A workspace without a map (the design's Empty map): pick the agent, then Init with it.
fn init_detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, w: &Workspace) {
    let ctx = ui.ctx().clone();
    let origin = ui.cursor().min.y;
    ui.add_space(52.0);
    ui.label(egui::RichText::new("NO MAP YET").font(mono(9.0)).color(s.accent));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(w.name()).font(semibold(26.0)).color(s.text));
    ui.add_space(6.0);
    let meta = [w.path.clone().unwrap_or_else(|| "no folder".to_string()), branch_label(w), agents_line(w)].join("  ·  ");
    ui.add(egui::Label::new(egui::RichText::new(meta).font(mono(11.0)).color(s.muted)).truncate());
    ui.add_space(12.0);
    ui.scope(|ui| {
        ui.set_max_width(560.0);
        let text = match w.map_state() {
            MapState::Empty => {
                "A document exists, but its map is empty. Init asks one agent to read the repository, name the areas and draw the first overview. Pick who: the agent already here, or a new CLI."
            }
            _ => {
                "No document yet. Init opens a live design and asks one agent to read the repository, name the areas and draw the first overview. Pick who: the agent already here, or a new CLI."
            }
        };
        let mut job = egui::text::LayoutJob::single_section(
            text.to_string(),
            egui::TextFormat { font_id: regular(12.5), color: s.muted, line_height: Some(18.0), ..Default::default() },
        );
        job.wrap.max_width = ui.available_width();
        ui.label(job);
    });

    let busy = app.ws.busy.clone();
    let busy_here = busy.as_ref().is_some_and(|b| b.for_workspace(&w.id));
    let targets = agent_menu::targets_of(w.agents_detail.clone(), &app.ws.clis);
    ui.add_space(10.0);
    ui.label(egui::RichText::new("AGENT FOR THIS MAP").font(mono(9.0)).color(s.muted));
    ui.add_space(6.0);
    if targets.is_empty() {
        paragraph(
            ui,
            "No agent is in this workspace and no agent CLI is installed to start one. Install codex, claude or kimi, then Refresh.",
            11.0,
            s.muted,
        );
    } else if let Some(target) = init_agent_button(ui, &mut app.ws, &w.id, &targets, busy.is_none()) {
        app.ws.target = Some(target);
    }
    let target = app.ws.target.clone();

    ui.add_space(16.0);
    if let Some(agent) = turned_busy(w, target.as_ref()) {
        // The second step: the picked agent is now busy; say with what, and let the human decide.
        let task = agent.title.as_deref().map(agent_task).filter(|task| !task.is_empty());
        let line = match task {
            Some(task) => format!("{} turned {} after you picked it: {task}", display_kind(&agent.kind), agent.status.trim()),
            None => format!("{} turned {} after you picked it.", display_kind(&agent.kind), agent.status.trim()),
        };
        ui.add(egui::Label::new(egui::RichText::new(line).font(mono(10.0)).color(s.accent)).wrap());
        ui.add_space(10.0);
        let mut anyway = false;
        let mut other = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            anyway = init_button(ui, s, Id::new(("init_anyway", &w.id)), &format!("Ask {} anyway", display_kind(&agent.kind)), busy.is_none());
            other = ghost_button(ui, s, Id::new(("init_other", &w.id)), "Pick another agent", busy.is_none(), false).clicked();
        });
        if anyway {
            app.ws.act(&ctx, &w.id, true);
        } else if other {
            app.ws.pick_another(w);
        }
    } else {
        let setup = !app.ws.agent_tools_missing.is_empty();
        let label = match busy.as_ref() {
            None if setup => "Set up agent tools & Init".to_string(),
            _ => action_label(w.map_state(), w, busy.as_ref()),
        };
        let enabled = busy.is_none() && !targets.is_empty() && target.is_some();
        if init_button(ui, s, Id::new(("workspace_action", &w.id)), &label, enabled) {
            app.ws.act(&ctx, &w.id, true);
        }
        ui.add_space(12.0);
        let hint = if busy_here {
            match &busy {
                Some(Busy::Open { target: Some(Target::New { kind }), .. }) => {
                    format!("the node is starting {}; the map appears as it draws", display_kind(kind))
                }
                Some(Busy::Open { target: Some(Target::Existing { kind, .. }), .. }) => {
                    format!("the node is asking {}; the map appears as it draws", display_kind(kind))
                }
                _ => "the node is starting the agent; the map appears as it draws".to_string(),
            }
        } else if setup {
            // A running agent loaded its MCP servers when it started: the node starts a new one.
            format!(
                "Installs what agents need first ({}), then starts a new agent: one already running can't load them.",
                app.ws.agent_tools_missing.join(", ")
            )
        } else {
            "One button, one menu: busy agents are disabled in the list.".to_string()
        };
        if !hint.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(hint).font(mono(10.0)).color(s.muted)).wrap());
        }
    }
    if let Some(error) = app.ws.error.clone() {
        ui.add_space(16.0);
        if error_card(ui, s, &error, app.ws.busy.is_some()) {
            app.ws.retry(&ctx);
        }
    }
    // The foot note sits low on the screen, as the design places it.
    let foot_at = origin + 432.0;
    let gap = (foot_at - ui.cursor().min.y).max(28.0);
    ui.add_space(gap);
    ui.scope(|ui| {
        ui.set_max_width(600.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "The agent button opens the same menu as the composer — search, a scrolling list and new CLIs. Pick an idle agent, press Init.",
                )
                .font(mono(9.0))
                .color(s.muted),
            )
            .wrap(),
        );
    });
}

/// AGENT FOR THIS MAP: the composer's agent button and menu, with busy agents disabled; the
/// pick, if any.
fn init_agent_button(ui: &mut Ui, ws: &mut State, workspace: &str, targets: &[crate::compose::TargetInfo], enabled: bool) -> Option<Target> {
    let label = agent_menu::button_label(targets, ws.target.as_ref(), "Choose an agent");
    let width = agent_menu::button_width(ui, &label, 220.0, 360.0);
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            agent_menu::button(ui, Id::new(("init_agent", workspace)), targets, ws.target.as_ref(), &label, vec2(width, 34.0), ws.agent_menu_open)
        })
        .inner;
    if response.clicked() {
        ws.agent_menu_open = !ws.agent_menu_open;
        if ws.agent_menu_open {
            ws.agent_menu.opened();
        }
    }
    let options = agent_menu::Options { disable_busy: true, ..agent_menu::Options::default() };
    agent_menu::popup(
        ui,
        Id::new(("init_agent_menu", workspace)),
        &response,
        &mut ws.agent_menu_open,
        &mut ws.agent_menu,
        targets,
        ws.target.as_ref(),
        options,
    )
}

/// The accent Init button: 34 pt tall, 16 pt of padding.
fn init_button(ui: &mut Ui, s: &Skin, id: Id, label: &str, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(12.0)) + 32.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(110.0), 34.0), Sense::hover());
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let fill = if !enabled {
        s.soft
    } else if resp.hovered() {
        s.accent_hover
    } else {
        s.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    if !enabled {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    }
    ui.painter().text(
        pos2(rect.min.x + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        semibold(12.0),
        if enabled { Color32::WHITE } else { s.faint },
    );
    focus_ring(ui, s, rect, &resp, 6);
    resp.clicked() && enabled
}

/// The action's label, including what the service is doing while it works.
fn action_label(kind: MapState, w: &Workspace, busy: Option<&Busy>) -> String {
    if let Some(Busy::Open { workspace, init, target: asking }) = busy
        && workspace == &w.id
    {
        if *init {
            return match asking {
                Some(Target::New { kind }) => format!("Starting {}…", display_kind(kind)),
                Some(Target::Existing { kind, .. }) => format!("Asking {}…", display_kind(kind)),
                None => "Starting the agent…".to_string(),
            };
        }
        return "Opening…".to_string();
    }
    // The agent is on its own button above; the action stays `Init map`.
    kind.action().to_string()
}

fn explanation(kind: MapState) -> &'static str {
    match kind {
        MapState::Map => "The map is attached to the workspace, not to a link. Open it, or ask the agents working there.",
        MapState::Empty => {
            "A document exists but its map is still empty. Init starts an agent with the mindmap skill; it reads the repository and draws the first overview."
        }
        MapState::None => {
            "No document yet. Init opens a live design and starts an agent with the mindmap skill; it reads the repository and draws the first overview."
        }
    }
}

/// `/root/code/mindmap   ·   main   ·   2 agents working`
fn meta_line(w: &Workspace) -> String {
    [w.path.clone().unwrap_or_else(|| "no folder".to_string()), branch_label(w), agents_line(w).replace(" · ", " ")].join("   ·   ")
}

fn agents_line(w: &Workspace) -> String {
    if w.agents == 0 {
        return "no agent".to_string();
    }
    let count = format!("{} agent{}", w.agents, if w.agents == 1 { "" } else { "s" });
    let status = w.agent_status.trim();
    if status.is_empty() || status.eq_ignore_ascii_case("unknown") { count } else { format!("{count} · {status}") }
}

/// MACHINE: the machine the list belongs to, Refresh, and the way to manage machines.
fn machine_block(app: &mut FigApp, ui: &mut Ui, s: &Skin) {
    let ctx = ui.ctx().clone();
    let eyebrow_rect = eyebrow(ui, s, "MACHINE");
    let refresh = Rect::from_min_size(pos2(ui.max_rect().max.x - 72.0, eyebrow_rect.min.y - 6.0), vec2(72.0, 24.0));
    let busy = app.ws.busy.is_some();
    if outline_button_at(ui, s, Id::new("workspaces_refresh"), refresh, "Refresh", !busy) {
        app.ws.refresh(&ctx);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let (name, meta) = {
        let host = app.ws.hosts.selected.clone().unwrap_or_else(|| crate::node_hosts::resolve(&service_addr()));
        let node = app.ws.node_version.as_deref().map(|v| format!("node {v}"));
        let mut parts = vec![match &host.ssh {
            Some(target) => format!("ssh · {target}"),
            None => "local".to_string(),
        }];
        parts.extend(node);
        parts.push("shared Herdr".to_string());
        (host.name, parts.join("   ·   "))
    };
    #[cfg(target_arch = "wasm32")]
    let (name, meta) = ("This computer".to_string(), "local".to_string());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(name).font(semibold(13.0)).color(s.text));
    ui.add_space(5.0);
    ui.label(egui::RichText::new(meta).font(mono(8.5)).color(s.faint));
    ui.add_space(9.0);
    if link(ui, s, Id::new("workspaces_manage_machines"), "Manage machines…", s.muted) {
        crate::machine_modal::open(app);
    }
}

/// AGENTS · N WORKING, then one line per agent: kind, what it is doing, and its state.
fn agents_block(ui: &mut Ui, s: &Skin, w: &Workspace) {
    let working = w.agents_detail.iter().filter(|a| is_working(&a.status)).count();
    let head = if working > 0 {
        format!("AGENTS   ·   {working} WORKING")
    } else if w.agents > 0 {
        format!("AGENTS   ·   {}", w.agents)
    } else {
        "AGENTS".to_string()
    };
    eyebrow(ui, s, &head);
    ui.add_space(8.0);
    if w.agents_detail.is_empty() {
        let text = if w.agents == 0 { "No agents in this workspace." } else { "The workspace has agents; the node did not name them." };
        paragraph(ui, text, 11.0, s.muted);
        return;
    }
    for agent in &w.agents_detail {
        agent_row(ui, s, agent);
    }
}

fn agent_row(ui: &mut Ui, s: &Skin, agent: &AgentDetail) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let status = agent.status.trim();
    let tint = status_color(status, s);
    ui.painter().circle_filled(pos2(rect.min.x + 3.0, rect.min.y + 9.0), 3.0, tint);
    let shown_status = if status.is_empty() || status.eq_ignore_ascii_case("unknown") { "" } else { status };
    let status_w = text_width(ui, shown_status, mono(8.5));
    ui.painter().text(pos2(rect.max.x, rect.min.y + 6.0), Align2::RIGHT_TOP, shown_status, mono(8.5), tint);
    let room = (rect.width() - 12.0 - status_w - 16.0).max(40.0);
    let name = widgets::elide_font(ui, display_kind(&agent.kind), room, semibold(11.0));
    ui.painter().text(pos2(rect.min.x + 12.0, rect.min.y), Align2::LEFT_TOP, name, semibold(11.0), s.text);
    if let Some(task) = agent.title.as_deref().map(agent_task).filter(|task| !task.is_empty()) {
        let task = widgets::elide_font(ui, &task, room, mono(8.5));
        ui.painter().text(pos2(rect.min.x + 12.0, rect.min.y + 16.0), Align2::LEFT_TOP, task, mono(8.5), s.faint);
    }
}

/// What an agent says it is doing, from its pane title: spinners and icons dropped, and for a
/// `repo | Ready | task | branch` title just the task.
pub(crate) fn agent_task(title: &str) -> String {
    let title = title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let parts: Vec<&str> = title.split(" | ").map(str::trim).collect();
    let task = if parts.len() >= 3 { parts.get(2).copied().unwrap_or(title) } else { title };
    task.to_lowercase()
}

fn is_working(status: &str) -> bool {
    matches!(status.trim().to_ascii_lowercase().as_str(), "working" | "busy" | "running" | "thinking")
}

fn status_color(status: &str, s: &Skin) -> Color32 {
    if is_working(status) {
        return s.map_ink;
    }
    match status.trim().to_ascii_lowercase().as_str() {
        "blocked" | "error" | "failed" => s.danger,
        _ => s.faint,
    }
}

/// LAST SYNC: the newest Mindmap-Sync commit on the workspace's branch.
fn sync_block(ui: &mut Ui, s: &Skin, w: &Workspace) {
    eyebrow(ui, s, "LAST SYNC");
    ui.add_space(4.0);
    let line = match (&w.last_sync, w.map_state()) {
        (Some(sync), _) => {
            let mut parts = vec![sync.commit.clone()];
            if !sync.subject.trim().is_empty() {
                parts.push(short_subject(&sync.subject));
            }
            if let Some(ago) = ago(&sync.at) {
                parts.push(ago);
            }
            parts.join("   ·   ")
        }
        (None, MapState::Map) => "not synced yet".to_string(),
        (None, _) => "no map yet".to_string(),
    };
    ui.add(egui::Label::new(egui::RichText::new(line).font(mono(9.0)).color(s.muted)).truncate());
}

/// A commit subject without its `Area:` prefix, cut to a few words.
fn short_subject(subject: &str) -> String {
    let subject = subject.split_once(": ").map_or(subject, |(_, rest)| rest).trim();
    let mut words: Vec<&str> = subject.split_whitespace().take(6).collect();
    if let Some(last) = words.last_mut() {
        *last = last.trim_end_matches([',', ';', ':']);
    }
    words.join(" ").to_lowercase()
}

/// `2 h ago` from an ISO 8601 date, against the system clock.
fn ago(at: &str) -> Option<String> {
    let then = parse_iso(at)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let secs = (now - then).max(0);
    Some(match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    })
}

/// Seconds since the epoch for `YYYY-MM-DDTHH:MM:SS±HH:MM` (or `Z`).
fn parse_iso(at: &str) -> Option<i64> {
    let at = at.trim();
    let num = |range: std::ops::Range<usize>| at.get(range).and_then(|v| v.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let zone = at.get(19..).unwrap_or("");
    let offset = if zone.is_empty() || zone.starts_with('Z') {
        0
    } else {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let oh = zone.get(1..3).and_then(|v| v.parse::<i64>().ok())?;
        let om = zone.get(4..6).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        sign * (oh * 3600 + om * 60)
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// A soft rounded note at the foot of the detail.
fn hint(ui: &mut Ui, s: &Skin, text: &str) {
    egui::Frame::NONE
        .fill(s.selected)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 29 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            paragraph(ui, text, 11.0, s.muted);
        });
}

/// A text link; true when clicked.
fn link(ui: &mut Ui, s: &Skin, id: Id, label: &str, ink: Color32) -> bool {
    let size = vec2(text_width(ui, label, regular(10.5)), 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    a11y::button(&resp, label);
    let ink = if resp.hovered() { s.text } else { ink };
    ui.painter().text(pos2(rect.min.x, rect.min.y + 3.0), Align2::LEFT_TOP, label, regular(10.5), ink);
    focus_ring(ui, s, rect.expand(2.0), &resp, 4);
    resp.clicked()
}

fn eyebrow(ui: &mut Ui, s: &Skin, text: &str) -> Rect {
    ui.label(egui::RichText::new(text).font(mono(9.0)).color(s.muted)).rect
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn error_card(ui: &mut Ui, s: &Skin, text: &str, busy: bool) -> bool {
    let mut retry = false;
    egui::Frame::NONE
        .fill(s.danger_bg)
        .stroke(Stroke::new(1.0, with_alpha(s.danger, 70)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(text).font(regular(12.0)).color(s.danger)).wrap());
            ui.add_space(8.0);
            retry = secondary_button_alloc(ui, s, Id::new("workspaces_error_retry"), "Retry", !busy);
        });
    retry
}

/// The dark primary button: 32 pt tall, 20 pt of padding either side.
fn primary_button(ui: &mut Ui, s: &Skin, id: Id, label: &str, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(11.0)) + 40.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(110.0), 32.0), Sense::hover());
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let hot = enabled && resp.hovered();
    let fill = if !enabled {
        s.soft
    } else if hot {
        s.primary_hover
    } else {
        s.primary
    };
    let ink = if enabled { s.on_primary } else { s.faint };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    if !enabled {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    }
    ui.painter().text(pos2(rect.min.x + 20.0, rect.center().y), Align2::LEFT_CENTER, label, semibold(11.0), ink);
    focus_ring(ui, s, rect, &resp, 6);
    resp.clicked() && enabled
}

/// The white outlined button beside the primary one; `menu` adds the arrow of a picker.
fn ghost_button(ui: &mut Ui, s: &Skin, id: Id, label: &str, enabled: bool, menu: bool) -> Response {
    let arrow = if menu { 18.0 } else { 0.0 };
    let width = text_width(ui, label, semibold(11.0)) + 32.0 + arrow;
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(110.0), 32.0), Sense::hover());
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let hot = enabled && menu && resp.hovered();
    ui.painter().rect_filled(rect, CornerRadius::same(6), if hot { s.soft } else { s.panel });
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    let ink = if enabled { s.text } else { s.faint };
    ui.painter().text(pos2(rect.min.x + 16.0, rect.center().y), Align2::LEFT_CENTER, label, semibold(11.0), ink);
    if menu {
        let c = pos2(rect.max.x - 18.0, rect.center().y + 0.5);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![pos2(c.x - 3.5, c.y - 2.0), pos2(c.x + 3.5, c.y - 2.0), pos2(c.x, c.y + 2.5)],
            if enabled { s.muted } else { s.faint },
            Stroke::NONE,
        ));
    }
    focus_ring(ui, s, rect, &resp, 6);
    resp
}

/// A small outlined button at a fixed place (Refresh).
fn outline_button_at(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let hot = enabled && resp.hovered();
    ui.painter().rect_filled(rect, CornerRadius::same(6), if hot { s.soft } else { s.panel });
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, label, semibold(10.0), if enabled { s.text } else { s.faint });
    focus_ring(ui, s, rect, &resp, 6);
    resp.clicked() && enabled
}

fn divider(ui: &mut Ui, s: &Skin) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, s.border));
}

pub(crate) fn secondary_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let hot = enabled && resp.hovered();
    let ink = if enabled { if hot { s.accent } else { s.muted } } else { s.faint };
    if hot {
        ui.painter().rect_filled(rect, CornerRadius::same(5), s.soft);
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, if hot { s.accent } else { s.border }), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(11.0), ink);
    focus_ring(ui, s, rect, &resp, 5);
    resp.clicked() && enabled
}

/// A secondary button laid out in the current vertical flow; true when clicked.
fn secondary_button_alloc(ui: &mut Ui, s: &Skin, id: Id, label: &str, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(11.0)) + 20.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    secondary_button(ui, s, id, rect, label, enabled)
}

pub(crate) fn focus_ring(ui: &Ui, s: &Skin, rect: Rect, resp: &Response, radius: u8) {
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(radius), Stroke::new(2.0, s.accent), StrokeKind::Outside);
    }
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// `a` moved `by` (0…1) of the way to `b`.
fn mix(a: Color32, b: Color32, by: f32) -> Color32 {
    let by = by.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * by).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod branch_transition_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inactive_branch_waits_for_busy_request_then_reopens_only_on_its_selected_host() {
        let node = crate::node::SessionRef {
            address: "ssh:work".into(),
            session: "old-branch".into(),
            workspace: "w1".into(),
            path: "/repo".into(),
            branch: "main".into(),
        };
        let inactive = json!({"event":"session","session":node.session,"metadata":{"active":false}});
        let mut state = State { busy: Some(Busy::List), ..State::default() };
        state.queue_branch_reopen(&node.address, &node.address, &inactive, Some(&node));
        assert!(state.take_branch_reopen(&node.address, Some(&node)).is_none());
        assert!(state.pending_reopen.is_some());
        state.busy = None;
        let reopen = state.take_branch_reopen(&node.address, Some(&node));
        assert_eq!(reopen.as_ref().map(|node| (node.address.as_str(), node.workspace.as_str())), Some(("ssh:work", "w1")));
        assert!(state.take_branch_reopen(&node.address, Some(&node)).is_none());

        state.queue_branch_reopen("ssh:other", &node.address, &inactive, Some(&node));
        assert!(state.pending_reopen.is_none());
        state.queue_branch_reopen(&node.address, &node.address, &inactive, Some(&node));
        assert!(state.take_branch_reopen("ssh:other", Some(&node)).is_none());
        assert!(state.pending_reopen.is_none());
    }
}

#[cfg(test)]
mod detail_tests {
    use super::*;

    fn agent(pane: &str, kind: &str, status: &str) -> AgentDetail {
        AgentDetail { pane: pane.into(), kind: kind.into(), status: status.into(), ..AgentDetail::default() }
    }

    #[test]
    fn a_listing_says_what_init_sets_up_and_an_older_node_says_nothing() {
        let reply: ServiceReply =
            serde_json::from_value(serde_json::json!({"ok": true, "agent_tools_missing": ["skill for claude", "MCP for codex"]})).unwrap();
        assert_eq!(reply.agent_tools_missing, ["skill for claude", "MCP for codex"]);
        let older: ServiceReply = serde_json::from_value(serde_json::json!({"ok": true, "clis": ["codex"]})).unwrap();
        assert!(older.agent_tools_missing.is_empty(), "a node that doesn't say is ready");
    }

    #[test]
    fn init_offers_the_agents_here_then_new_clis_and_disables_busy_ones() {
        let w = Workspace {
            agents_detail: vec![agent("w:p1", "kimi", "idle"), agent("w:p2", "codex", "working"), agent("w:p3", "claude", "blocked")],
            ..Workspace::default()
        };
        // The composer's menu: the agents here first, then the CLIs a new agent can start.
        let targets = agent_menu::targets_of(w.agents_detail.clone(), &["claude".into(), "codex".into()]);
        let lines: Vec<String> = targets.iter().map(|info| info.target.line()).collect();
        assert_eq!(lines, ["pane:w:p1", "pane:w:p2", "pane:w:p3", "kind:claude", "kind:codex"]);
        let disabled: Vec<bool> = targets.iter().map(|info| !info.new_agent && agent_menu::is_busy(&info.status)).collect();
        assert_eq!(disabled, [false, true, true, false, false], "working and blocked agents are disabled in Init's menu");
        let first = default_target(&w, &["claude".into()]);
        assert_eq!(
            first,
            Some(Target::Existing { pane: "w:p1".into(), kind: "kimi".into(), status: "idle".into() }),
            "a busy agent is never the default"
        );
        assert_eq!(agent_menu::button_label(&targets, first.as_ref(), "Choose an agent"), "Pane 1 · Kimi · p1 · idle");
        assert_eq!(action_label(MapState::Empty, &w, None), "Init map", "the agent is on its own button");
    }

    #[test]
    fn pick_another_agent_returns_to_the_default_and_opens_the_menu() {
        let w = Workspace {
            id: "w".into(),
            agents_detail: vec![agent("w:p1", "kimi", "working"), agent("w:p2", "codex", "idle")],
            ..Workspace::default()
        };
        let mut ws = State {
            rows: vec![w.clone()],
            target: Some(Target::Existing { pane: "w:p1".into(), kind: "kimi".into(), status: "idle".into() }),
            ..State::default()
        };
        assert!(turned_busy(&w, ws.target.as_ref()).is_some(), "the second step shows");
        ws.pick_another(&w);
        assert_eq!(ws.target.as_ref().map(Target::line).as_deref(), Some("pane:w:p2"));
        assert!(ws.agent_menu_open, "the agent menu opens on the idle agent");
        ws.select("w");
        assert!(!ws.agent_menu_open, "selecting a workspace closes it");
    }

    #[test]
    fn a_picked_agent_that_turns_busy_needs_a_second_step() {
        let mut w = Workspace { agents_detail: vec![agent("w:p1", "kimi", "idle")], ..Workspace::default() };
        let picked = Target::Existing { pane: "w:p1".into(), kind: "kimi".into(), status: "idle".into() };
        assert!(turned_busy(&w, Some(&picked)).is_none());
        w.agents_detail[0].status = "working".into();
        assert!(turned_busy(&w, Some(&picked)).is_some());
        assert!(turned_busy(&w, Some(&Target::New { kind: "kimi".into() })).is_none());
        // With every agent busy, Init falls back to starting a CLI.
        assert_eq!(default_target(&w, &["claude".into()]), Some(Target::New { kind: "claude".into() }));
    }

    #[test]
    fn agent_task_drops_spinners_and_keeps_the_task_of_a_status_title() {
        assert_eq!(agent_task("✦ 🐋 Finalize Pane 2 Claude work and install"), "finalize pane 2 claude work and install");
        assert_eq!(agent_task("mindmap | Ready | Sync backend mindmap | main"), "sync backend mindmap");
        assert_eq!(agent_task("Alzette mindmap overview"), "alzette mindmap overview");
        assert_eq!(agent_task("⠇ "), "");
    }

    #[test]
    fn iso_dates_read_with_their_offset() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(parse_iso("2026-10-24T16:20:36+02:00"), Some(1_792_851_636));
        assert_eq!(parse_iso("not a date"), None);
        assert_eq!(parse_iso("2026-10"), None);
    }

    #[test]
    fn the_sync_line_shortens_the_subject() {
        assert_eq!(
            short_subject("Mindmap client: onboarding, machine and workspace setup, worktree status bar"),
            "onboarding, machine and workspace setup, worktree"
        );
        assert_eq!(short_subject("node cleanup"), "node cleanup");
    }

    #[test]
    fn rows_say_no_agent_or_count_and_state() {
        let mut w = Workspace { agents: 0, ..Workspace::default() };
        assert_eq!(agents_line(&w), "no agent");
        w.agents = 2;
        w.agent_status = "working".into();
        assert_eq!(agents_line(&w), "2 agents · working");
        assert_eq!(pane_agents(&w), "2 agents");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn machines_describe_where_they_are() {
        let local = crate::node_hosts::Host { name: "This computer".into(), address: "127.0.0.1:47822".into(), ssh: None };
        let remote = crate::node_hosts::Host { name: "befree".into(), address: "ssh:befree".into(), ssh: Some("befree".into()) };
        assert_eq!(machine_state(&local, true, false, 3, None), "local · 3 workspaces");
        assert_eq!(machine_state(&remote, true, false, 1, Some("0.1.0")), "ssh · connected · node 0.1.0");
        assert_eq!(machine_state(&remote, true, true, 0, None), "ssh · connecting…");
        assert_eq!(machine_state(&remote, false, false, 0, None), "ssh · befree");
    }

    #[test]
    fn a_new_list_selects_the_focused_workspace() {
        let mut state = State::default();
        let (tx, rx) = mpsc::channel();
        state.rx = Some(rx);
        let reply = ServiceReply {
            ok: true,
            workspaces: Some(vec![
                Workspace { id: "w1".into(), ..Workspace::default() },
                Workspace { id: "w2".into(), focused: true, ..Workspace::default() },
            ]),
            node: Some("0.1.0".into()),
            ..ServiceReply::default()
        };
        assert!(tx.send((Kind::List, Ok(reply))).is_ok());
        state.poll();
        assert_eq!(state.selected.as_deref(), Some("w2"));
        assert_eq!(state.node_version.as_deref(), Some("0.1.0"));
    }
}
