//! The Workspaces screen: every Herdr workspace on the project host, with or without a map.
//!
//! Background workers share one versioned node connection per machine. Opening a workspace
//! returns its session id, worktree path and branch; the engine joins through NodeLiveLink.
//! A successful welcome opens the document with its existing page tree.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{OnceLock, RwLock};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::Deserialize;

use crate::FigApp;
use crate::a11y;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// Where the node listens unless `MINDMAP_NODE` or `--node` says otherwise.
const DEFAULT_SERVICE: &str = "127.0.0.1:47822";

/// The list's width (the detail gets the rest of the window).
const SIDEBAR: f32 = 340.0;
/// The thin bar at the top of the window.
const HEADER_H: f32 = 52.0;
/// One workspace row.
const ROW_H: f32 = 76.0;
/// The Init split button's height and the chevron segment's width.
const SPLIT_H: f32 = 34.0;
const CHEVRON_W: f32 = 32.0;
/// The agent menu's width (it shrinks to fit a narrow window).
const PICKER_W: f32 = 360.0;

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
    /// Its lifecycle state (`idle`, `working`, `blocked`, `done`, …).
    #[serde(default)]
    pub status: String,
    /// Focused in the user's Herdr UI.
    #[serde(default)]
    pub focused: bool,
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

    /// What the detail says about the choice: `kimi · idle` or `start claude`.
    fn summary(&self) -> String {
        match self {
            Target::Existing { kind, status, .. } => {
                let kind = display_kind(kind);
                let status = status.trim();
                if status.is_empty() { kind.to_string() } else { format!("{kind} · {status}") }
            }
            Target::New { kind } => format!("start {}", display_kind(kind)),
        }
    }

    /// Whether this is the agent at `pane`.
    fn is_at(&self, pane: &str) -> bool {
        matches!(self, Target::Existing { pane: chosen, .. } if chosen == pane)
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

    fn name(&self) -> &str {
        if self.label.trim().is_empty() { &self.id } else { &self.label }
    }

    /// Whether Init has an agent to ask or a CLI to start.
    fn has_agents(&self, clis: &[String]) -> bool {
        !self.agents_detail.is_empty()
            || self.agents > 0
            || self.agent_names.iter().any(|name| !name.trim().is_empty())
            || clis.iter().any(|kind| !kind.trim().is_empty())
    }
}

/// The target a freshly selected workspace starts on: its focused agent, else a non-blocked one,
/// else any agent; with no agent, the first installed CLI; with neither, none (the service chooses).
fn default_target(w: &Workspace, clis: &[String]) -> Option<Target> {
    let agent = w
        .agents_detail
        .iter()
        .find(|agent| agent.focused)
        .or_else(|| w.agents_detail.iter().find(|agent| !is_blocked(&agent.status)))
        .or_else(|| w.agents_detail.first());
    if let Some(agent) = agent {
        return Some(Target::Existing { pane: agent.pane.clone(), kind: agent.kind.clone(), status: agent.status.clone() });
    }
    clis.iter().map(|kind| kind.trim()).find(|kind| !kind.is_empty()).map(|kind| Target::New { kind: kind.to_string() })
}

fn is_blocked(status: &str) -> bool {
    status.trim().eq_ignore_ascii_case("blocked")
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
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    branch: String,
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
    /// The last error, shown in muted red with a Retry.
    pub error: Option<String>,
    /// The request in flight, if any.
    pub busy: Option<Busy>,
    /// The agent CLIs installed on the project host, from the last list.
    pub clis: Vec<String>,
    /// What Init should use for the selected workspace; `None` lets the service choose.
    pub target: Option<Target>,
    /// The Init agent menu is open (it belongs to the screen, not to a row).
    picker_open: bool,
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
}

impl State {
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
        self.picker_open = false;
        self.retry = Some(Retry::List);
        self.start(ctx, Request::List);
    }

    /// Select `workspace` and put the Init target back on its first agent (or first CLI).
    pub fn select(&mut self, workspace: &str) {
        self.selected = Some(workspace.to_string());
        self.picker_open = false;
        self.error = None;
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
        self.picker_open = false;
        self.error = None;
        // Open and Init both publish the workspace's map; remember which one for the join.
        self.join_workspace = Some(workspace.to_string());
        self.retry = Some(Retry::Action { workspace: workspace.to_string(), init, target: target.clone() });
        self.start(ctx, Request::Act { workspace: workspace.to_string(), init, target });
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

    /// Keep the chosen target when a new list still offers it; refresh its status; otherwise fall
    /// back to the workspace's first agent or CLI.
    fn reconcile_target(&mut self) {
        let Some(id) = self.selected.as_deref() else {
            self.target = None;
            self.picker_open = false;
            return;
        };
        let Some(w) = self.rows.iter().find(|w| w.id == id) else {
            self.selected = None;
            self.target = None;
            self.picker_open = false;
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
        self.busy = None;
        let (kind, result) = received;
        match kind {
            Kind::List => match result {
                Ok(reply) if reply.ok => {
                    self.rows = reply.workspaces.unwrap_or_default();
                    self.clis = reply.clis;
                    if self.selected.is_none() {
                        self.target = None;
                        self.picker_open = false;
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

/// A compact monitor-ish palette over the app's tokens: warm paper for the map client, the same
/// accent as the map bar. Dark mode keeps the tokens' surfaces and lightens the inks.
struct Skin {
    bg: Color32,
    panel: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    faint: Color32,
    accent: Color32,
    accent_hover: Color32,
    selected: Color32,
    soft: Color32,
    map_ink: Color32,
    danger: Color32,
    danger_bg: Color32,
}

impl Skin {
    fn of(t: &Tokens) -> Skin {
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
            }
        } else {
            let danger = Color32::from_rgb(0x9E, 0x3B, 0x2C);
            Skin {
                bg: Color32::from_rgb(0xF4, 0xF4, 0xF2),
                panel: t.panel,
                border: t.border,
                text: t.text,
                muted: t.text_secondary,
                faint: t.text_tertiary,
                accent: Color32::from_rgb(0xB5, 0x45, 0x2F),
                accent_hover: Color32::from_rgb(0xC2, 0x55, 0x3E),
                selected: Color32::from_rgb(0xF1, 0xEC, 0xE7),
                soft: Color32::from_rgb(0xF8, 0xF6, 0xF3),
                map_ink: Color32::from_rgb(0x2F, 0x6F, 0x6A),
                danger,
                danger_bg: with_alpha(danger, 16),
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
    // The host controls precede the central panel, so paint their surface too rather than
    // letting the window's clear color show behind the workspace palette's text.
    ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, skin.panel);
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(skin.bg)).show(ui, |ui| {
        let rect = ui.max_rect();
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H.min(rect.height())));
        let body = Rect::from_min_max(pos2(rect.min.x, head.max.y), rect.max);
        header(app, ui, &skin, head);
        // The list keeps its width; the detail takes the rest, but never less than 320 px.
        let side = SIDEBAR.min((body.width() - 320.0).max(240.0));
        let list = Rect::from_min_size(body.min, vec2(side, body.height()));
        let detail_rect = Rect::from_min_max(pos2(list.max.x, body.min.y), body.max);
        sidebar(app, ui, &skin, list);
        detail(app, ui, &skin, detail_rect);
        ui.painter().line_segment([pos2(list.max.x, body.min.y), pos2(list.max.x, body.max.y)], Stroke::new(1.0, skin.border));
    });
    // The New workspace modal first, so the Add machine modal it opens draws above it.
    crate::new_workspace::show(app, ui);
    crate::machine_modal::show(app, ui);
}

/// The togglable workspace pane beside an open map. It lists the same Herdr workspaces as the
/// browser; clicking one switches the map in place instead of leaving the canvas.
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
    let skin = Skin::of(&Tokens::get(&ctx));
    ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, skin.panel);
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.add_space(8.0);
        machines(app, ui, &skin);
    }
    let active = app.active_workspace.clone();
    let rows = app.ws.rows.clone();
    let groups = grouped(&rows);
    let busy = app.ws.busy.is_some();
    let error = app.ws.error.clone();
    let current = active.as_deref();

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        eyebrow(ui, &skin, "WORKSPACES");
        ui.label(egui::RichText::new(groups.len().to_string()).font(mono(9.5)).color(skin.faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            if ui.add(egui::Button::new(egui::RichText::new("Refresh").font(regular(10.0)).color(skin.muted)).frame(false)).clicked() {
                app.ws.refresh(&ctx);
            }
        });
    });
    ui.add_space(8.0);
    ui.painter()
        .line_segment([ui.max_rect().left_top() + vec2(0.0, 34.0), ui.max_rect().right_top() + vec2(0.0, 34.0)], Stroke::new(1.0, skin.border));

    egui::ScrollArea::vertical().id_salt("workspace_pane_scroll").show(ui, |ui| {
        if new_workspace_row(ui, &skin) {
            crate::new_workspace::open(app);
        }
        let mut chosen = None;
        for (base, branches) in &groups {
            let is_active = current.is_some_and(|id| branches.iter().any(|b| b.id == id));
            let is_busy = busy && app.ws.selected.as_deref() == Some(base.id.as_str());
            if let Some(choice) = row(ui, &skin, base, is_active, is_busy) {
                chosen = Some(choice);
            }
        }
        if let Some(choice) = chosen {
            match choice {
                Chosen::Select(id) => {
                    let kind = rows.iter().find(|w| w.id == id).map(|w| w.map_state());
                    if kind == Some(MapState::Map) {
                        app.ws.act(&ctx, &id, false);
                    } else {
                        app.ws.select(&id);
                        app.ui.home = true;
                    }
                }
                Chosen::Act { workspace, init } => {
                    if init {
                        app.ws.select(&workspace);
                        app.ui.home = true;
                    } else {
                        app.ws.act(&ctx, &workspace, false);
                    }
                }
            }
        }
    });

    let foot = egui::Rect::from_min_size(pos2(ui.max_rect().left(), ui.max_rect().bottom() - 56.0), vec2(ui.max_rect().width(), 56.0));
    ui.painter().rect_filled(foot, CornerRadius::ZERO, skin.panel);
    ui.painter().line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, skin.border));
    if error.is_some() {
        ui.painter().text(pos2(foot.left() + 14.0, foot.top() + 14.0), Align2::LEFT_CENTER, "Could not reach the node.", mono(9.0), skin.accent);
    }
    if ui
        .put(
            egui::Rect::from_min_size(pos2(foot.left() + 12.0, foot.top() + 24.0), vec2(foot.width() - 24.0, 24.0)),
            egui::Button::new(egui::RichText::new("All workspaces").font(regular(10.5)).color(skin.muted)).frame(false),
        )
        .clicked()
    {
        app.ui.home = true;
    }
    // The same modals as the full screen, so the pane can create and add machines too.
    crate::new_workspace::show(app, ui);
    crate::machine_modal::show(app, ui);
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
            live.status == figcraft_engine::live::Status::Live && live.info.as_ref().is_some_and(|info| info.link == route)
        });
        if let Some(index) = existing {
            app.session.active = index;
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
        return Err("Open a connected node workspace first.".into());
    }
    crate::node::SessionRef::parse(&live.info.as_ref().ok_or("The node session is not ready.")?.link)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn update_active(app: &mut FigApp) {
    app.active_workspace = active_node(app).ok().map(|target| target.workspace);
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
                }
                if active.as_ref().is_some_and(|node| node.address == address) {
                    app.compose.targets_at = -10.0;
                }
            }
            Some("job") => crate::sync::node_event(app, ctx, &address, event),
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
/// The machines block at the top of the Workspaces sidebar: the saved machines, what setup is
/// doing, and the way to add or manage one. Adding a machine lives in the machine modal; the
/// SSH target is never typed into a page header.
#[cfg(not(target_arch = "wasm32"))]
fn machines(app: &mut FigApp, ui: &mut Ui, s: &Skin) {
    if app.ws.pending_live.is_some() {
        ui.label("Joining the workspace\u{2026}");
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        eyebrow(ui, s, "MACHINES");
    });
    let ready = crate::node_hosts::poll(&mut app.ws.hosts);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.vertical(|ui| crate::node_hosts::chips(ui, &mut app.ws.hosts));
    });
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let manage = ui.add(egui::Button::new(egui::RichText::new("Manage machines\u{2026}").font(regular(10.0)).color(s.muted)).frame(false));
        a11y::link(&manage, "Manage machines");
        if manage.clicked() {
            crate::machine_modal::open(app);
        }
        ui.add_space(8.0);
        let add = ui.add(egui::Button::new(egui::RichText::new("+ Add machine").font(semibold(10.0)).color(s.accent)).frame(false));
        a11y::link(&add, "Add machine");
        if add.clicked() {
            crate::machine_modal::open(app);
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.vertical(|ui| crate::node_hosts::status(ui, &mut app.ws.hosts));
    });
    if let Some(host) = ready {
        use_host(app, ui.ctx(), &host);
    }
}

/// Use `host` on the Workspaces screen: drop the old host's answers and list its workspaces.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_host(app: &mut FigApp, ctx: &egui::Context, host: &crate::node_hosts::Host) {
    set_service(&host.address);
    // Discard old-host responses before requesting the new host's workspaces.
    app.ws.rx = None;
    app.ws.busy = None;
    app.ws.selected = None;
    app.ws.rows.clear();
    app.ws.clis.clear();
    app.ws.target = None;
    app.ws.join = None;
    app.ws.join_workspace = None;
    app.ws.hosts.selected = Some(host.clone());
    app.ws.refresh(ctx);
}

fn header(app: &mut FigApp, ui: &mut Ui, s: &Skin, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, s.panel);
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, s.border));
    let y = rect.center().y;
    let x = rect.min.x + 24.0;
    p.text(pos2(x, y), Align2::LEFT_CENTER, "Mindmap", semibold(13.0), s.text);
    let word = text_width(ui, "Mindmap", semibold(13.0));
    p.text(pos2(x + word + 8.0, y), Align2::LEFT_CENTER, "·", regular(13.0), s.faint);
    let dot = text_width(ui, "·", regular(13.0));
    p.text(pos2(x + word + 8.0 + dot + 8.0, y), Align2::LEFT_CENTER, "Workspaces", regular(13.0), s.text);

    // Right: the service address (quiet), Refresh, and a way back to an open map.
    let mut right = rect.max.x - 20.0;
    let address = service_addr();
    let address_w = text_width(ui, &address, mono(10.5));
    ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, &address, mono(10.5), s.faint);
    right -= address_w + 16.0;

    let list_busy = matches!(app.ws.busy, Some(Busy::List));
    let refresh = if list_busy { "Refreshing…" } else { "Refresh" };
    let refresh_w = text_width(ui, refresh, semibold(11.0)) + 20.0;
    let refresh_rect = Rect::from_min_size(pos2(right - refresh_w, y - 13.0), vec2(refresh_w, 26.0));
    if secondary_button(ui, s, Id::new("workspaces_refresh"), refresh_rect, refresh, !list_busy) {
        app.ws.refresh(ui.ctx());
    }
    right = refresh_rect.min.x - 8.0;

    if app.has_files() {
        let back = "Back to map";
        let back_w = text_width(ui, back, semibold(11.0)) + 20.0;
        let back_rect = Rect::from_min_size(pos2(right - back_w, y - 13.0), vec2(back_w, 26.0));
        if secondary_button(ui, s, Id::new("workspaces_back"), back_rect, back, true) {
            app.ui.home = false;
        }
    }
}

/// The repository a workspace belongs to; linked worktrees share their base repo's key.
fn repo_key(w: &Workspace) -> String {
    w.repo.clone().filter(|repo| !repo.trim().is_empty()).or_else(|| w.path.clone()).unwrap_or_else(|| w.id.clone())
}

/// One row per repository: the base checkout and its linked worktrees as branches.
fn grouped(rows: &[Workspace]) -> Vec<(Workspace, Vec<Workspace>)> {
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
fn branch_label(w: &Workspace) -> String {
    w.branch
        .clone()
        .filter(|branch| !branch.trim().is_empty())
        .or_else(|| w.path.as_deref().and_then(|path| Path::new(path).file_name()).map(|name| name.to_string_lossy().to_string()))
        .unwrap_or_else(|| w.name().to_string())
}

/// The "+ New workspace" row at the top of the list and the pane.
fn new_workspace_row(ui: &mut Ui, s: &Skin) -> bool {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    a11y::button(&resp, "New workspace");
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.soft);
    }
    let c = pos2(rect.min.x + 16.0, rect.center().y - 3.0);
    let stroke = Stroke::new(1.5, s.accent);
    ui.painter().line_segment([pos2(c.x - 4.0, c.y), pos2(c.x + 4.0, c.y)], stroke);
    ui.painter().line_segment([pos2(c.x, c.y - 4.0), pos2(c.x, c.y + 4.0)], stroke);
    ui.painter().text(pos2(rect.min.x + 28.0, rect.center().y - 3.0), Align2::LEFT_CENTER, "New workspace", semibold(11.5), s.muted);
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, s.border));
    resp.clicked()
}

fn sidebar(app: &mut FigApp, ui: &mut Ui, s: &Skin, rect: Rect) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, s.panel);
    let mut top = rect.min.y;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let area = Rect::from_min_size(rect.min, vec2(rect.width(), 150.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            ui.add_space(8.0);
            machines(app, ui, s);
            top = ui.min_rect().max.y + 8.0;
        });
    }
    let head = Rect::from_min_size(pos2(rect.min.x, top), vec2(rect.width(), 42.0));
    ui.painter().text(pos2(head.min.x + 16.0, head.center().y), Align2::LEFT_CENTER, "WORKSPACES", mono(9.5), s.faint);
    let groups = grouped(&app.ws.rows);
    let count = groups.len().to_string();
    ui.painter().text(pos2(head.max.x - 16.0, head.center().y), Align2::RIGHT_CENTER, count, mono(10.0), s.faint);
    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, s.border));

    let list = Rect::from_min_max(pos2(rect.min.x, head.max.y), rect.max);
    let ctx = ui.ctx().clone();
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        ui.set_clip_rect(list);
        egui::ScrollArea::vertical().id_salt("workspaces_list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if new_workspace_row(ui, s) {
                crate::new_workspace::open(app);
            }
            let (error, busy, started) = (app.ws.error.clone(), app.ws.busy.is_some(), app.ws.started);
            if app.ws.rows.is_empty() {
                empty_list(app, ui, s, &ctx, error.as_deref(), busy, started);
            } else {
                let mut chosen = None;
                for (base, branches) in &groups {
                    let selected = app.ws.selected.as_deref().is_some_and(|id| branches.iter().any(|b| b.id == id));
                    if let Some(c) = row(ui, s, base, selected, busy) {
                        chosen = Some(c);
                    }
                }
                match chosen {
                    Some(Chosen::Select(id)) => app.ws.select(&id),
                    Some(Chosen::Act { workspace, init }) => app.ws.act(&ctx, &workspace, init),
                    None => {}
                }
            }
        });
    });
}

/// A row asks for one thing: select its workspace, or open / initialise it.
enum Chosen {
    Select(String),
    Act { workspace: String, init: bool },
}

fn row(ui: &mut Ui, s: &Skin, w: &Workspace, selected: bool, busy: bool) -> Option<Chosen> {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    let kind = w.map_state();
    a11y::button(&resp, &format!("{} · {} · {}", w.name(), kind.badge(), agents_line(w)));
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.selected);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, s.accent);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.soft);
    }
    focus_ring(ui, s, rect, &resp, 0);

    // The badge, top right.
    let badge = kind.badge();
    let badge_font = semibold(9.0);
    let badge_w = text_width(ui, badge, badge_font.clone()) + 12.0;
    let badge_rect = Rect::from_min_size(pos2(rect.max.x - 14.0 - badge_w, rect.min.y + 11.0), vec2(badge_w, 16.0));
    let tint = match kind {
        MapState::Map => s.map_ink,
        MapState::Empty => s.muted,
        MapState::None => s.accent,
    };
    ui.painter().rect_filled(badge_rect, CornerRadius::same(4), with_alpha(tint, if s.panel == Color32::WHITE { 18 } else { 36 }));
    ui.painter().rect_stroke(badge_rect, CornerRadius::same(4), Stroke::new(1.0, with_alpha(tint, 90)), StrokeKind::Inside);
    ui.painter().text(badge_rect.center(), Align2::CENTER_CENTER, badge, badge_font, tint);

    // The focused workspace in Herdr: a small dot beside its label.
    let label_x = if w.focused { rect.min.x + 24.0 } else { rect.min.x + 16.0 };
    if w.focused {
        ui.painter().circle_filled(pos2(rect.min.x + 12.0, rect.min.y + 15.0), 3.0, s.map_ink);
    }
    let name = w.name();
    let name_room = (badge_rect.min.x - 10.0 - label_x).max(40.0);
    let name_shown = widgets::elide_font(ui, name, name_room, semibold(13.0));
    ui.painter().text(pos2(label_x, rect.min.y + 15.0), Align2::LEFT_CENTER, &name_shown, semibold(13.0), s.text);

    let path = w.path.as_deref().or(w.repo.as_deref()).unwrap_or("no folder");
    let path_room = (rect.max.x - 14.0 - (rect.min.x + 16.0)).max(40.0);
    let path_shown = widgets::elide_start_font(ui, path, path_room, mono(10.5));
    ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y + 35.0), Align2::LEFT_CENTER, &path_shown, mono(10.5), s.muted);

    // The row's own Open / Init button, bottom right.
    let action = if kind == MapState::Map { "Open" } else { "Init" };
    let action_w = text_width(ui, action, semibold(10.5)) + 20.0;
    let action_rect = Rect::from_min_size(pos2(rect.max.x - 14.0 - action_w, rect.min.y + 44.0), vec2(action_w, 22.0));
    let action_resp = ui.interact(action_rect, ui.id().with(("workspace_row_action", &w.id)), if busy { Sense::hover() } else { Sense::click() });
    a11y::button(&action_resp, action);
    let hot = !busy && action_resp.hovered();
    let (action_ink, action_border) = if busy {
        (s.faint, s.border)
    } else if hot || selected {
        (s.accent, s.accent)
    } else {
        (s.muted, s.border)
    };
    if hot {
        ui.painter().rect_filled(action_rect, CornerRadius::same(4), s.soft);
    }
    ui.painter().rect_stroke(action_rect, CornerRadius::same(4), Stroke::new(1.0, action_border), StrokeKind::Inside);
    ui.painter().text(action_rect.center(), Align2::CENTER_CENTER, action, semibold(10.5), action_ink);
    focus_ring(ui, s, action_rect, &action_resp, 4);

    let agents_room = (action_rect.min.x - 8.0 - (rect.min.x + 16.0)).max(40.0);
    let agents = widgets::elide_font(ui, &agents_line(w), agents_room, regular(10.5));
    ui.painter().text(pos2(rect.min.x + 16.0, rect.min.y + 56.0), Align2::LEFT_CENTER, &agents, regular(10.5), s.muted);

    let cut = name_shown != name || path_shown != path || agents != agents_line(w);
    let resp = if cut {
        resp.on_hover_text(format!("{name}\n{path}\n{}", agents_line(w)))
    } else if w.focused {
        resp.on_hover_text("Focused in Herdr")
    } else {
        resp
    };
    let on_button = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| action_rect.contains(p));
    let mut chosen = None;
    if resp.clicked() && !on_button {
        chosen = Some(Chosen::Select(w.id.clone()));
    }
    if resp.double_clicked() && !on_button {
        chosen = Some(Chosen::Act { workspace: w.id.clone(), init: kind != MapState::Map });
    }
    if action_resp.clicked() && !busy {
        chosen = Some(Chosen::Act { workspace: w.id.clone(), init: kind != MapState::Map });
    }
    chosen
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

fn detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, rect: Rect) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, s.bg);
    let selected = app.ws.selected.as_deref().and_then(|id| app.ws.rows.iter().find(|w| w.id == id)).cloned();
    let inner = Rect::from_min_max(rect.min + vec2(32.0, 26.0), rect.max - vec2(32.0, 24.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_clip_rect(rect);
        egui::ScrollArea::vertical().id_salt("workspaces_detail").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(inner.width().min(720.0));
            match &selected {
                Some(w) => workspace_detail(app, ui, s, w),
                None => intro(app, ui, s),
            }
        });
    });
}

fn intro(app: &mut FigApp, ui: &mut Ui, s: &Skin) {
    ui.add_space(36.0);
    eyebrow(ui, s, "WORKSPACES");
    ui.add_space(10.0);
    ui.label(egui::RichText::new("Herdr workspaces are the projects.").font(semibold(27.0)).color(s.text));
    ui.add_space(8.0);
    paragraph(ui, "A map is optional.", 14.0, s.muted);
    ui.add_space(16.0);
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
    ui.add_space(26.0);
    let maps = app.ws.rows.iter().filter(|w| w.map_state() == MapState::Map).count();
    let summary = match (app.ws.busy.is_some(), app.ws.rows.len()) {
        (true, 0) => "Reading the workspaces from the node…".to_string(),
        (_, 0) if app.ws.started => "No workspaces listed yet.".to_string(),
        (_, 0) => String::new(),
        (_, n) => format!("{n} workspace{} · {maps} with a map", if n == 1 { "" } else { "s" }),
    };
    if !summary.is_empty() {
        ui.label(egui::RichText::new(summary).font(mono(10.5)).color(s.faint));
    }
}

fn workspace_detail(app: &mut FigApp, ui: &mut Ui, s: &Skin, w: &Workspace) {
    let kind = w.map_state();
    let ctx = ui.ctx().clone();
    eyebrow(ui, s, "SELECTED WORKSPACE");
    ui.add_space(6.0);
    ui.label(egui::RichText::new(w.name()).font(semibold(28.0)).color(s.text));
    ui.add_space(8.0);
    let path = w.path.as_deref().unwrap_or("no folder");
    ui.add(egui::Label::new(egui::RichText::new(path).font(mono(11.0)).color(s.muted)).wrap());
    ui.add_space(3.0);
    ui.label(egui::RichText::new(meta_line(w)).font(regular(11.0)).color(s.faint));
    ui.add_space(18.0);
    paragraph(ui, explanation(kind), 13.0, s.muted);

    // What Init will do: ask the chosen agent, start a chosen CLI, or let the service choose.
    ui.add_space(20.0);
    let busy = app.ws.busy.clone();
    let busy_here = busy.as_ref().is_some_and(|b| b.for_workspace(&w.id));
    let init = kind != MapState::Map;
    let target = if init { app.ws.target.clone() } else { None };
    let can_init = !init || w.has_agents(&app.ws.clis);
    let enabled = busy.is_none() && can_init;
    let menu_available = !w.agents_detail.is_empty() || app.ws.clis.iter().any(|kind| !kind.trim().is_empty());
    let label = action_label(kind, w, target.as_ref(), busy.as_ref());
    let split_id = Id::new(("workspace_action", &w.id));
    let mut run = false;
    let mut toggle = false;
    let mut anchor = None;
    ui.horizontal(|ui| {
        if init {
            let width = text_width(ui, &label, semibold(12.5)) + 34.0;
            let (rect, _) = ui.allocate_exact_size(vec2(width + CHEVRON_W, SPLIT_H), Sense::hover());
            let split = split_button(ui, s, split_id, rect, &label, enabled, menu_available);
            run = split.primary;
            toggle = split.chevron;
            anchor = Some(split.response);
        } else {
            let width = text_width(ui, &label, semibold(12.5)) + 40.0;
            let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
            if primary_button(ui, s, split_id, rect, &label, enabled) {
                run = true;
            }
        }
        if w.agents > 0 {
            ui.add_space(10.0);
            agents_pill(ui, s, w);
        }
    });
    if toggle {
        app.ws.picker_open = !app.ws.picker_open;
    }
    if run {
        app.ws.picker_open = false;
        app.ws.act(&ctx, &w.id, init);
    } else if init
        && app.ws.picker_open
        && let Some(anchor) = anchor
    {
        agent_picker(app, ui, s, w, &anchor);
    }
    if init {
        if !can_init {
            ui.add_space(9.0);
            paragraph(
                ui,
                "No agent is in this workspace and no agent CLI is installed to start one. Install codex, claude or kimi, then Refresh.",
                11.0,
                s.muted,
            );
        } else if let Some(target) = target.as_ref() {
            ui.add_space(9.0);
            let line = match target {
                Target::Existing { .. } => format!("{} · already in the workspace", target.summary()),
                Target::New { .. } => format!("{} · a new agent in the workspace", target.summary()),
            };
            ui.label(egui::RichText::new(line).font(mono(10.5)).color(s.faint));
        } else if let Some(agent) = agent_name(w) {
            ui.add_space(9.0);
            ui.label(egui::RichText::new(format!("asks {agent} · already in the workspace")).font(mono(10.5)).color(s.faint));
        }
    }
    if busy_here {
        ui.add_space(9.0);
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
            paragraph(ui, &line, 11.5, s.faint);
        }
    }

    if let Some(error) = app.ws.error.clone() {
        ui.add_space(18.0);
        if error_card(ui, s, &error, app.ws.busy.is_some()) {
            app.ws.retry(&ctx);
        }
    }

    let branches = grouped(&app.ws.rows).into_iter().find(|(_, all)| all.iter().any(|b| b.id == w.id)).map(|(_, all)| all).unwrap_or_default();
    if branches.len() > 1 {
        ui.add_space(20.0);
        eyebrow(ui, s, "BRANCHES");
        ui.add_space(6.0);
        for branch in &branches {
            let active = app.active_workspace.as_deref() == Some(branch.id.as_str());
            let kind = branch.map_state();
            let label = format!("{}  ·  {}", branch_label(branch), kind.badge());
            let enabled = app.ws.busy.is_none();
            if secondary_button_alloc(ui, s, Id::new(("workspace_branch", &branch.id)), &label, enabled) && !active {
                if kind == MapState::Map {
                    app.ws.act(&ctx, &branch.id, false);
                } else {
                    app.ws.select(&branch.id);
                }
            }
        }
    }
    agents_block(ui, s, w);
    details_block(ui, s, w);
}

/// The action's label, including what the service is doing while it works.
fn action_label(kind: MapState, w: &Workspace, target: Option<&Target>, busy: Option<&Busy>) -> String {
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
    match kind {
        MapState::Map => kind.action().to_string(),
        MapState::Empty | MapState::None => match target {
            Some(Target::Existing { kind, .. }) => format!("Init with {}", display_kind(kind)),
            Some(Target::New { kind }) => format!("Start {}", display_kind(kind)),
            None => "Init map".to_string(),
        },
    }
}

fn explanation(kind: MapState) -> &'static str {
    match kind {
        MapState::Map => "The map is attached to this workspace, not to a link. Open it to read, select and comment.",
        MapState::Empty => {
            "A document exists but its map is still empty. Init starts an agent with the mindmap skill; it reads the repository and draws the first overview."
        }
        MapState::None => {
            "No document yet. Init opens a live design and starts an agent with the mindmap skill; it reads the repository and draws the first overview."
        }
    }
}

fn meta_line(w: &Workspace) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(repo) = w.repo.as_deref().filter(|r| !r.trim().is_empty()) {
        parts.push(repo);
    }
    if w.worktree {
        parts.push("worktree");
    }
    let agents = agents_line(w);
    parts.push(&agents);
    parts.join("  ·  ")
}

fn agents_line(w: &Workspace) -> String {
    if w.agents == 0 {
        return "no agents".to_string();
    }
    let count = format!("{} agent{}", w.agents, if w.agents == 1 { "" } else { "s" });
    let status = w.agent_status.trim();
    if status.is_empty() || status.eq_ignore_ascii_case("unknown") { count } else { format!("{count} · {status}") }
}

fn agent_name(w: &Workspace) -> Option<&str> {
    w.agent_names.iter().map(String::as_str).find(|name| !name.trim().is_empty())
}

fn agents_pill(ui: &mut Ui, s: &Skin, w: &Workspace) {
    let label = agents_line(w);
    let width = text_width(ui, &label, regular(10.5)) + 22.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, s.border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, &label, regular(10.5), s.muted);
}

fn agents_block(ui: &mut Ui, s: &Skin, w: &Workspace) {
    ui.add_space(26.0);
    eyebrow(ui, s, "AGENTS");
    ui.add_space(8.0);
    egui::Frame::NONE.fill(s.soft).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 2.0;
        if !w.agents_detail.is_empty() {
            for agent in &w.agents_detail {
                agent_row(ui, s, display_kind(&agent.kind), &agent.status, agent.focused);
            }
            let unnamed = (w.agents as usize).saturating_sub(w.agents_detail.len());
            if unnamed > 0 {
                paragraph(ui, &format!("+{unnamed} other{}", if unnamed == 1 { "" } else { "s" }), 10.5, s.faint);
            }
        } else if w.agent_names.is_empty() {
            paragraph(
                ui,
                if w.agents == 0 { "No agents in this workspace." } else { "The workspace has agents; the node did not name them." },
                11.5,
                s.muted,
            );
            if w.map_state() != MapState::Map {
                ui.add_space(2.0);
                paragraph(ui, "Init starts the selected CLI; choose another with the chevron.", 11.0, s.faint);
            }
        } else {
            for name in &w.agent_names {
                agent_row(ui, s, display_kind(name), &w.agent_status, false);
            }
            let unnamed = (w.agents as usize).saturating_sub(w.agent_names.len());
            if unnamed > 0 {
                paragraph(ui, &format!("+{unnamed} more"), 10.5, s.faint);
            }
        }
    });
}

fn agent_row(ui: &mut Ui, s: &Skin, name: &str, status: &str, focused: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let dot = pos2(rect.min.x + 3.0, rect.center().y);
    ui.painter().circle_filled(dot, 3.0, status_color(status, s));
    if focused {
        ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, s.map_ink));
    }
    ui.painter().text(pos2(rect.min.x + 13.0, rect.center().y), Align2::LEFT_CENTER, name, semibold(11.5), s.text);
    if !status.trim().is_empty() && !status.eq_ignore_ascii_case("unknown") {
        ui.painter().text(pos2(rect.max.x, rect.center().y), Align2::RIGHT_CENTER, status, mono(10.0), s.faint);
    }
}

fn status_color(status: &str, s: &Skin) -> Color32 {
    match status.to_ascii_lowercase().as_str() {
        "working" | "busy" | "running" | "thinking" => s.accent,
        "blocked" | "error" | "failed" => s.danger,
        "idle" | "done" | "ready" | "waiting" => s.map_ink,
        _ => s.faint,
    }
}

fn details_block(ui: &mut Ui, s: &Skin, w: &Workspace) {
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(repo) = w.repo.as_deref().filter(|r| !r.trim().is_empty()) {
        rows.push(("Repository", repo.to_string()));
    }
    if w.worktree {
        rows.push(("Checkout", "linked worktree".to_string()));
    }
    if let Some(tabs) = w.tabs {
        rows.push(("Tabs", tabs.to_string()));
    }
    if let Some(panes) = w.panes {
        rows.push(("Panes", panes.to_string()));
    }
    if rows.is_empty() {
        return;
    }
    ui.add_space(26.0);
    eyebrow(ui, s, "WORKSPACE");
    ui.add_space(8.0);
    for (label, value) in &rows {
        meta_row(ui, s, label, value);
    }
}

fn meta_row(ui: &mut Ui, s: &Skin, label: &str, value: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, label, mono(10.0), s.faint);
    let room = (rect.width() - 110.0).max(40.0);
    let shown = widgets::elide_start_font(ui, value, room, mono(10.5));
    ui.painter().text(pos2(rect.max.x, rect.center().y), Align2::RIGHT_CENTER, &shown, mono(10.5), s.muted);
    if shown != value {
        let _ = resp.on_hover_text(value);
    }
}

fn eyebrow(ui: &mut Ui, s: &Skin, text: &str) {
    ui.label(egui::RichText::new(text).font(mono(9.5)).color(s.faint));
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

fn primary_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    let hot = enabled && resp.hovered();
    let fill = if !enabled {
        s.soft
    } else if hot {
        s.accent_hover
    } else {
        s.accent
    };
    let ink = if enabled { Color32::WHITE } else { s.faint };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(12.5), ink);
    focus_ring(ui, s, rect, &resp, 6);
    resp.clicked() && enabled
}

/// What the Init split button did this frame.
struct SplitClick {
    primary: bool,
    chevron: bool,
    /// The whole split, for anchoring the agent menu below it.
    response: Response,
}

/// The primary Init button with its narrower, darker chevron segment. Both are 34 px tall.
fn split_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool, menu: bool) -> SplitClick {
    let seam = rect.max.x - CHEVRON_W;
    let main = Rect::from_min_max(rect.min, pos2(seam, rect.max.y));
    let chevron = Rect::from_min_max(pos2(seam, rect.min.y), rect.max);
    let whole = ui.interact(rect, id.with("whole"), Sense::hover());
    let main_resp = ui.interact(main, id.with("init"), if enabled { Sense::click() } else { Sense::hover() });
    let menu_enabled = enabled && menu;
    let menu_resp = ui.interact(chevron, id.with("menu"), if menu_enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&main_resp, label);
    a11y::button(&menu_resp, "Choose the agent for Init");

    let hot = enabled && main_resp.hovered();
    let menu_hot = menu_enabled && menu_resp.hovered();
    let main_fill = if !enabled {
        s.soft
    } else if hot {
        s.accent_hover
    } else {
        s.accent
    };
    let menu_fill = if !menu_enabled {
        s.soft
    } else if menu_hot {
        darken(s.accent_hover, 0.90)
    } else {
        darken(s.accent, 0.82)
    };
    let main_ink = if enabled { Color32::WHITE } else { s.faint };
    let menu_ink = if menu_enabled { Color32::WHITE } else { s.faint };
    ui.painter().rect_filled(main, CornerRadius { nw: 6, ne: 0, sw: 6, se: 0 }, main_fill);
    ui.painter().rect_filled(chevron, CornerRadius { nw: 0, ne: 6, sw: 0, se: 6 }, menu_fill);
    ui.painter().text(main.center(), Align2::CENTER_CENTER, label, semibold(12.5), main_ink);

    // The downward chevron is drawn, not a glyph, so it never depends on the font.
    let c = chevron.center() + vec2(0.0, 0.5);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(c.x - 4.0, c.y - 2.0), pos2(c.x + 4.0, c.y - 2.0), pos2(c.x, c.y + 2.5)],
        menu_ink,
        Stroke::NONE,
    ));
    focus_ring(ui, s, main, &main_resp, 6);
    focus_ring(ui, s, chevron, &menu_resp, 6);
    SplitClick { primary: enabled && main_resp.clicked(), chevron: menu_enabled && menu_resp.clicked(), response: whole }
}

/// The Init agent picker: an egui popup anchored below the split button. Clicking a row closes it,
/// and egui's popup handling closes it on a click outside or Escape; it re-aligns to stay in the
/// window. No native window is involved.
fn agent_picker(app: &mut FigApp, ui: &mut Ui, s: &Skin, w: &Workspace, anchor: &Response) {
    let mut open = app.ws.picker_open;
    let chosen = app.ws.target.clone();
    let clis = app.ws.clis.clone();
    let ctx = ui.ctx().clone();
    let id = Id::new(("workspace_action", &w.id)).with("menu").with("popup");
    let width = PICKER_W.min((ctx.content_rect().width() - 24.0).max(120.0));
    let frame = egui::Frame::NONE
        .fill(s.panel)
        .stroke(Stroke::new(1.0, s.border))
        .corner_radius(CornerRadius::same(8))
        .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(38) })
        .inner_margin(egui::Margin::symmetric(12, 11));
    let mut picked = None;
    egui::Popup::new(id, ctx, anchor, anchor.layer_id)
        .width(width)
        .gap(6.0)
        .align(egui::RectAlign::BOTTOM_START)
        .frame(frame)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picked = picker_body(ui, s, w, chosen.as_ref(), &clis);
        });
    if let Some(target) = picked {
        app.ws.target = Some(target);
        open = false;
    }
    app.ws.picker_open = open;
}

/// The picker's contents; a click on an existing agent or a CLI returns the target it picked.
fn picker_body(ui: &mut Ui, s: &Skin, w: &Workspace, chosen: Option<&Target>, clis: &[String]) -> Option<Target> {
    let mut picked = None;
    ui.set_width(ui.available_width());
    ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
    eyebrow(ui, s, "AGENTS IN THIS WORKSPACE");
    ui.add_space(1.0);
    if w.agents_detail.is_empty() {
        paragraph(ui, "No agents in this workspace yet.", 11.0, s.muted);
    } else {
        for agent in &w.agents_detail {
            let selected = chosen.is_some_and(|target| target.is_at(&agent.pane));
            if picker_agent_row(ui, s, agent, selected) {
                picked = Some(Target::Existing { pane: agent.pane.clone(), kind: agent.kind.clone(), status: agent.status.clone() });
            }
        }
    }

    ui.add_space(5.0);
    divider(ui, s);
    ui.add_space(5.0);
    eyebrow(ui, s, "START A NEW AGENT");
    ui.add_space(1.0);
    let clis: Vec<&str> = clis.iter().map(|kind| kind.trim()).filter(|kind| !kind.is_empty()).collect();
    if clis.is_empty() {
        paragraph(ui, "No agent CLIs installed on the project host.", 11.0, s.muted);
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for kind in clis {
                let selected = matches!(chosen, Some(Target::New { kind: chosen }) if chosen.trim().eq_ignore_ascii_case(kind));
                if picker_chip(ui, s, kind, selected) {
                    picked = Some(Target::New { kind: kind.to_string() });
                }
            }
        });
    }
    ui.add_space(9.0);
    paragraph(ui, "The agent reads the repo and draws; 6–12 areas, every card sourced.", 10.5, s.faint);
    picked
}

fn picker_agent_row(ui: &mut Ui, s: &Skin, agent: &AgentDetail, chosen: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let kind = display_kind(&agent.kind);
    let status = agent.status.trim();
    a11y::button(&resp, &if status.is_empty() { kind.to_string() } else { format!("{kind} · {status}") });
    let hot = resp.hovered() || resp.has_focus();
    if chosen || hot {
        let fill = if chosen { s.selected } else { s.soft };
        ui.painter().rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(5), fill);
    }
    let dot = pos2(rect.min.x + 8.0, rect.center().y);
    ui.painter().circle_filled(dot, 3.0, status_color(status, s));
    if agent.focused {
        ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, s.map_ink));
    }
    ui.painter().text(pos2(rect.min.x + 18.0, rect.center().y), Align2::LEFT_CENTER, kind, semibold(11.5), s.text);
    if !status.is_empty() && !status.eq_ignore_ascii_case("unknown") {
        let trail = if chosen { 22.0 } else { 8.0 };
        ui.painter().text(pos2(rect.max.x - trail, rect.center().y), Align2::RIGHT_CENTER, status, mono(10.0), s.faint);
    }
    if chosen {
        let c = pos2(rect.max.x - 10.0, rect.center().y);
        let stroke = Stroke::new(1.6, s.accent);
        ui.painter().line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.0, c.y + 3.5)], stroke);
        ui.painter().line_segment([pos2(c.x - 1.0, c.y + 3.5), pos2(c.x + 4.5, c.y - 4.0)], stroke);
    }
    focus_ring(ui, s, rect.shrink2(vec2(4.0, 1.0)), &resp, 5);
    resp.clicked()
}

fn picker_chip(ui: &mut Ui, s: &Skin, kind: &str, chosen: bool) -> bool {
    let label = display_kind(kind);
    let width = text_width(ui, label, semibold(10.5)) + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    a11y::button(&resp, label);
    let hot = resp.hovered() || resp.has_focus();
    let fill = if chosen {
        with_alpha(s.accent, if s.panel == Color32::WHITE { 12 } else { 28 })
    } else if hot {
        s.soft
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    let border = if chosen {
        s.accent
    } else if hot {
        with_alpha(s.accent, 140)
    } else {
        s.border
    };
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = if chosen || hot { s.accent } else { s.text };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(10.5), ink);
    focus_ring(ui, s, rect, &resp, 5);
    resp.clicked()
}

fn divider(ui: &mut Ui, s: &Skin) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, s.border));
}

fn darken(color: Color32, factor: f32) -> Color32 {
    let factor = factor.clamp(0.0, 1.0);
    Color32::from_rgb((color.r() as f32 * factor) as u8, (color.g() as f32 * factor) as u8, (color.b() as f32 * factor) as u8)
}

fn secondary_button(ui: &mut Ui, s: &Skin, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
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

fn focus_ring(ui: &Ui, s: &Skin, rect: Rect, resp: &Response, radius: u8) {
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
