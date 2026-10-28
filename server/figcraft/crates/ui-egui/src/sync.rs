//! Sync: the map's drift from the last sync, the code drift since it, and one action.
//!
//! A sync covers the current page and its subpages by default, or every page. A page sync is
//! compared with each page's own last sync, and its code side is limited to the paths the page's
//! cards cite; the service keeps one baseline per page.
//!
//! The button is a floating pill at the map's top right, never a mode: it opens a widget with the
//! design changes the agent is about to implement, the code that changed since the last sync, an
//! agent target and an optional note. A sync runs on the node; while it is in flight the
//! widget (and the button, when the widget is closed) shows the flight, the map stays editable, and
//! a thread polls `sync_status` once a second until the service reports `done` or `failed`.
//!
//! Every request is newline JSON over plain TCP on a named thread of its own. The snapshot is built
//! from the open document each time a diff or sync is asked for, so the widget never talks about a
//! map the human is no longer looking at.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::agent_menu;
use crate::compose::TargetInfo;
use crate::theme::{regular, semibold};
use crate::widgets;
use crate::workspaces::{self, Target};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// The most pages one snapshot may carry (the service rejects more).
const MAX_PAGES: usize = 500;
/// The most nodes one snapshot may carry (the service rejects more).
const MAX_NODES: usize = 5_000;
/// The note is kept well under the service's prompt limit.
const MAX_NOTE_CHARS: usize = 2_000;
/// The widget's width.
const WIDGET_W: f32 = 376.0;
/// The widget's inner padding.
const PAD: f32 = 19.0;
/// How many names and files each list shows before `+N more`.
const NAMES: usize = 4;
const FILES: usize = 4;

/// The warm map palette, shared with the composer and the map bar.
const PANEL: Color32 = Color32::WHITE;
const SOFT: Color32 = Color32::from_rgb(0xF8, 0xF6, 0xF3);
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const ACCENT_HOVER: Color32 = Color32::from_rgb(0xC2, 0x55, 0x3E);
const MAP_INK: Color32 = Color32::from_rgb(0x2F, 0x6F, 0x6A);
const DANGER: Color32 = Color32::from_rgb(0x9E, 0x3B, 0x2C);

/// A diff runs `git` on the project host and may read a large repository.
#[cfg(not(target_arch = "wasm32"))]
const DIFF_TIMEOUT: Duration = Duration::from_secs(30);
/// Starting a sync may have to start an agent and accept a first-run prompt.
#[cfg(not(target_arch = "wasm32"))]
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
/// One status poll asks Herdr; it is quick, but the host may be busy.
#[cfg(not(target_arch = "wasm32"))]
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
/// A reply larger than this is refused rather than read into memory.
/// How often the status thread asks while a job is in flight.
#[cfg(not(target_arch = "wasm32"))]
const POLL_EVERY: Duration = Duration::from_secs(1);
/// Network errors in a row before the poll gives up and says so.
#[cfg(not(target_arch = "wasm32"))]
const POLL_MISSES: u8 = 3;

/// The snapshot's page: what the service stores and compares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapPage {
    pub id: u64,
    pub name: String,
    /// The parent page of a subpage (`mindmap.parent_page`), so the service knows the tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
}

/// The snapshot's node: the card as the map shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapNode {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub page: u64,
    pub kind: String,
    pub text: String,
}

/// The map as it is now: pages and every card below them, depth first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub pages: Vec<SnapPage>,
    pub nodes: Vec<SnapNode>,
}

impl Snapshot {
    /// Walk `doc.pages`, then each page's children, capping at the service's limits.
    ///
    /// The walk is iterative and remembers visited nodes, so a malformed document with a cycle
    /// cannot loop or overflow the stack. Pages and cards beyond the caps are left out (the
    /// service rejects larger snapshots outright).
    pub fn of(doc: &Document) -> Snapshot {
        Snapshot::of_pages(doc, None)
    }

    /// Like [`Snapshot::of`], limited to `page` and its subpages when one is given.
    pub fn of_pages(doc: &Document, page: Option<NodeId>) -> Snapshot {
        let scope: Option<HashSet<NodeId>> = page.map(|page| {
            let mut pages: HashSet<NodeId> = crate::page_tree::descendants(doc, page).into_iter().collect();
            pages.insert(page);
            pages
        });
        let mut pages = Vec::new();
        let mut nodes = Vec::new();
        // The map's cards are its top-level frames. Nested frames are the card's internal layout,
        // and text layers are its body: both travel as the card's text, so the diff talks about
        // cards the human named, not every text layer in the document.
        for page_id in &doc.pages {
            if pages.len() >= MAX_PAGES || nodes.len() >= MAX_NODES {
                break;
            }
            if scope.as_ref().is_some_and(|scope| !scope.contains(page_id)) {
                continue;
            }
            let Some(page) = doc.node(*page_id) else { continue };
            let parent = crate::page_tree::parent(doc, *page_id).map(|parent| parent.0);
            pages.push(SnapPage { id: page_id.0, name: page.name.clone(), parent });
            for child in &page.children {
                if nodes.len() >= MAX_NODES {
                    break;
                }
                let Some(node) = doc.node(*child) else { continue };
                if !is_card(&node.kind) {
                    continue;
                }
                nodes.push(SnapNode {
                    id: child.0,
                    name: node.name.clone(),
                    parent: None,
                    page: page_id.0,
                    kind: kind_name(&node.kind).to_string(),
                    text: card_text(doc, *child),
                });
            }
        }
        Snapshot { pages, nodes }
    }
    /// Whether anything would differ between the two snapshots.
    pub fn differs(&self, other: &Snapshot) -> bool {
        self != other
    }
}

fn is_card(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Frame
            | NodeKind::Group
            | NodeKind::Section { .. }
            | NodeKind::Component { .. }
            | NodeKind::ComponentSet { .. }
            | NodeKind::Instance { .. }
    )
}

/// Every text layer below `root`, joined; the card's body for the diff.
fn card_text(doc: &Document, root: NodeId) -> String {
    let mut out = String::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        if let NodeKind::Text(text) = &node.kind {
            let part = text.characters.trim();
            if !part.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(part);
                if out.len() >= 4_000 {
                    out.truncate(4_000);
                    break;
                }
            }
        }
        for child in node.children.iter().rev() {
            stack.push(*child);
        }
    }
    out
}

/// The short kind name the service's snapshot stores.
fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Frame => "frame",
        NodeKind::Text(_) => "text",
        NodeKind::Group => "group",
        NodeKind::Rectangle => "rectangle",
        NodeKind::Ellipse { .. } => "ellipse",
        NodeKind::Vector { .. } => "vector",
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => "component",
        NodeKind::Instance { .. } => "instance",
        _ => "other",
    }
}

/// The last successful sync, as the service reports it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Last {
    #[serde(default)]
    pub exists: bool,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub at: String,
}

/// Deserialize `last` as `None` when the service says it does not exist.
fn last_or_none<'de, D>(deserializer: D) -> Result<Option<Last>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let last = Option::<Last>::deserialize(deserializer)?;
    Ok(last.and_then(|last| last.exists.then_some(last)))
}

/// One added or removed card in the design diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapCard {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub page: String,
}

/// One changed card in the design diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapChange {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

/// The design half of the diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapDiff {
    #[serde(default)]
    pub added: Vec<MapCard>,
    #[serde(default)]
    pub removed: Vec<MapCard>,
    #[serde(default)]
    pub changed: Vec<MapChange>,
}

impl MapDiff {
    /// The changes the widget counts and the button badges.
    pub fn total(&self) -> usize {
        self.added.len().saturating_add(self.removed.len()).saturating_add(self.changed.len())
    }
}

/// One changed file in the code diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct FileDelta {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub removed: u64,
}

/// One commit in the code diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CommitInfo {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub subject: String,
}

/// The code half of the diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CodeDiff {
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub files: Vec<FileDelta>,
    #[serde(default)]
    pub commits: Vec<CommitInfo>,
    #[serde(default)]
    pub note: Option<String>,
    /// The repository paths the files and commits are limited to; empty is the whole repository.
    #[serde(default)]
    pub scope: Vec<String>,
}

/// The service's answer to `diff`.
#[derive(Debug, Default, Deserialize)]
pub struct DiffReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, deserialize_with = "last_or_none")]
    pub last: Option<Last>,
    #[serde(default)]
    pub map: MapDiff,
    #[serde(default)]
    pub code: CodeDiff,
    /// The Sync or record already running for this session, if any: Sync waits for it.
    #[serde(default)]
    pub open: Option<Job>,
}

impl DiffReply {
    /// Whether a sync would have anything to apply or say.
    fn has_work(&self) -> bool {
        self.map.total() > 0 || !self.code.files.is_empty() || !self.code.commits.is_empty()
    }
}

/// One sync job the service reports.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub id: String,
    /// `in_flight`, `done` or `failed`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub started: String,
    #[serde(default)]
    pub agent: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
    /// What the job records: `sync`, or `explain` / `fix` for an agent's own record.
    #[serde(default, rename = "kind")]
    pub work: String,
}

impl Job {
    /// An agent's record of its own explanation or bug fix, not a Sync.
    pub fn is_record(&self) -> bool {
        matches!(self.work.trim(), "explain" | "fix")
    }

    /// The pane of the agent that opened the job, when it said.
    fn pane(&self) -> Option<&str> {
        self.agent.as_ref()?.get("pane")?.as_str().map(str::trim).filter(|pane| !pane.is_empty())
    }

    fn in_flight(&self) -> bool {
        self.state.trim().eq_ignore_ascii_case("in_flight")
    }

    fn failed(&self) -> bool {
        self.state.trim().eq_ignore_ascii_case("failed")
    }

    /// The agent kind the flight names: the service's, then the outcome's name.
    fn kind(&self) -> Option<String> {
        let agent = self.agent.as_ref()?;
        ["kind", "name"]
            .into_iter()
            .filter_map(|key| agent.get(key).and_then(Value::as_str))
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_string)
    }
}

/// The service's answer to `sync`.
#[derive(Debug, Default, Deserialize)]
pub struct SyncReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub job: Option<Job>,
    #[serde(default)]
    pub agent: Option<Value>,
}

/// The service's answer to `sync_status`.
#[derive(Debug, Default, Deserialize)]
pub struct StatusReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub job: Option<Job>,
    #[serde(default, deserialize_with = "last_or_none")]
    pub last: Option<Last>,
}

/// What a job thread sends back.
pub enum JobMsg {
    /// The `sync` request returned (or failed before it did).
    Started(Result<SyncReply, String>),
    /// One `sync_status` poll returned.
    Status(Result<StatusReply, String>),
}

/// The Sync button and widget's state.
#[derive(Default)]
pub struct State {
    /// The widget is showing.
    pub open: bool,
    /// Where the header's Sync button was drawn last frame (the first-map tooltip hangs under it).
    pub(crate) button_rect: Option<Rect>,
    /// The widget body's height last frame, so the viewport hugs it.
    content_h: f32,
    /// The widget opened this frame: the press that opened it is not a click outside.
    just_opened: bool,
    /// A diff or a sync request is on its thread (status polls are not "busy").
    pub busy: bool,
    /// The diff last read; replaced after a successful sync.
    pub diff: Option<DiffReply>,
    /// The last sync the service reported (also inside `diff`, kept while a refresh runs).
    pub last: Option<Last>,
    /// Why the last request failed, shown in the widget.
    pub error: Option<String>,
    /// The optional note sent with the next sync.
    pub note: String,
    /// The current or last sync job.
    pub job: Option<Job>,
    #[cfg(not(target_arch = "wasm32"))]
    node: Option<crate::node::SessionRef>,
    #[cfg(not(target_arch = "wasm32"))]
    pushed_jobs: Vec<Value>,
    /// The diff thread's answer.
    pub rx: Option<Receiver<Result<DiffReply, String>>>,
    /// The sync-start thread and the status poll thread's messages.
    pub job_rx: Option<Receiver<JobMsg>>,
    /// The map changed after the diff was read.
    pub dirty: bool,
    /// The snapshot the current diff was taken from.
    snapshot: Option<Snapshot>,
    /// The workspace this state belongs to.
    workspace: Option<String>,
    /// The document uid the diff belongs to.
    doc: Option<u64>,
    /// The design changes the current job was started with.
    applied: usize,
    /// A diff refresh is due after the job finished.
    refresh: bool,
    /// The sync request is on its thread; the job has not come back yet.
    starting: bool,
    /// The agent menu is open.
    menu_open: bool,
    /// The agent menu's search and arrow cursor.
    menu: crate::agent_menu::MenuState,
    /// Sync every page instead of the current page and its subpages.
    all_pages: bool,
    /// The page the diff was read for (`None` when it covers every page).
    page: Option<u64>,
    /// A cancel of an open record is on its thread.
    cancelling: bool,
    cancel_rx: Option<Receiver<Result<(), String>>>,
}

impl State {
    /// Whether a job is in flight (the button says `Syncing…`).
    fn in_flight(&self) -> bool {
        self.job.as_ref().is_some_and(Job::in_flight)
    }

    /// Whether the action button must stay disabled.
    fn locked(&self) -> bool {
        self.busy || self.starting || self.in_flight() || self.blocker().is_some()
    }

    /// A Sync or record running for this session that is not this widget's own flight: the node
    /// refuses a new Sync until it finishes or is cancelled.
    pub fn blocker(&self) -> Option<&Job> {
        let own = self.job.as_ref().filter(|job| job.in_flight()).map(|job| job.id.as_str());
        self.diff.as_ref()?.open.as_ref().filter(|job| job.in_flight() && Some(job.id.as_str()) != own)
    }

    /// The design changes the button badges, when a diff is loaded.
    fn changes(&self) -> Option<usize> {
        self.diff.as_ref().map(|diff| diff.map.total())
    }

    /// Whether a sync has something to do: a design or code change, or a note.
    fn has_work(&self) -> bool {
        !self.note.trim().is_empty() || self.diff.as_ref().is_some_and(DiffReply::has_work)
    }

    /// Forget everything about the previous workspace or document, keeping nothing in flight.
    fn reset(&mut self) {
        *self = State::default();
    }
}

/// The Sync action belongs to the map header: one compact button, no floating pill.
pub fn header_button(app: &mut FigApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    let syncing = app.sync.in_flight() || app.sync.starting;
    let enabled = app.active_workspace.is_some();
    let look = button_look(syncing, enabled, app.sync.changes());
    // `Sync` and, once a diff is read, the change count in a red badge; `Syncing…` in the accent.
    let font = semibold(10.5);
    let label_w = text_width(ui, look.label, font.clone());
    let badge = look.count.map(|count| count.to_string());
    // An 18 pt circle, widened into a pill for longer counts.
    let badge_w = badge.as_ref().map(|count| (text_width(ui, count, mono(8.0)) + 9.0).max(18.0));
    let width = (12.0 + label_w + badge_w.map_or(12.0, |w| 10.0 + w + 6.0)).max(52.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    app.sync.button_rect = Some(rect);
    let hot = enabled && response.hovered();
    ui.painter().rect_filled(rect, CornerRadius::same(8), if hot { SOFT } else { PANEL });
    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, look.stroke), StrokeKind::Inside);
    ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, look.label, font, look.ink);
    if let (Some(count), Some(badge_w)) = (badge, badge_w) {
        let pill = egui::Rect::from_min_size(pos2(rect.max.x - 6.0 - badge_w, rect.center().y - 9.0), vec2(badge_w, 18.0));
        ui.painter().rect_filled(pill, CornerRadius::same(9), ACCENT);
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, count, mono(8.0), Color32::WHITE);
    }
    a11y::button(&response, "Sync the map with the workspace");
    let response = response.on_hover_text("Sync the map with the workspace");
    if response.clicked() && enabled {
        open_panel(app, ctx);
    }
}

/// What the header button shows in each designed state.
#[derive(Debug, PartialEq, Eq)]
struct ButtonLook {
    label: &'static str,
    /// The change count badge, when a diff found changes.
    count: Option<usize>,
    ink: Color32,
    stroke: Color32,
}

/// Idle `Sync`; Changes `Sync` with its count; Syncing `Syncing…` in the accent; Disabled grey.
fn button_look(syncing: bool, enabled: bool, changes: Option<usize>) -> ButtonLook {
    if syncing {
        return ButtonLook { label: "Syncing…", count: None, ink: ACCENT, stroke: ACCENT };
    }
    if !enabled {
        return ButtonLook { label: "Sync", count: None, ink: FAINT, stroke: BORDER };
    }
    ButtonLook { label: "Sync", count: changes.filter(|n| *n > 0), ink: INK, stroke: BORDER }
}

/// Open the widget and read a fresh diff from the current map.
pub fn open_panel(app: &mut FigApp, ctx: &egui::Context) {
    app.sync.open = true;
    app.sync.just_opened = true;
    app.sync.menu_open = false;
    if app.sync.busy {
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else {
        app.toast("Open a workspace map to sync.");
        return;
    };
    match snapshot_now(app) {
        Ok((snapshot, page)) => start_diff(app, ctx, snapshot, page, workspace),
        Err(message) => {
            app.sync.open = false;
            app.toast(message);
        }
    }
}

/// Draw the Sync widget (the header button opens it).
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else {
        if app.sync.workspace.is_some() {
            app.sync.reset();
        }
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    let identity = workspaces::active_node(app).map(|node| format!("{}:{}", node.address, node.session)).unwrap_or_else(|_| workspace.clone());
    #[cfg(target_arch = "wasm32")]
    let identity = workspace.clone();
    if app.sync.workspace.as_deref() != Some(identity.as_str()) {
        app.sync.reset();
        app.sync.workspace = Some(identity);
    }
    let uid = app.session.ds().ok().map(|ds| ds.uid);
    if app.sync.doc != uid {
        // A different document: its diff is not this map's. A job already in flight belongs to
        // the workspace and keeps polling; only the diff is dropped.
        app.sync.doc = uid;
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.dirty = false;
        app.sync.error = None;
        app.sync.open = false;
        app.sync.menu_open = false;
    }

    // A page sync's diff belongs to the page it was read on: moving to another page reads that
    // page's diff instead. A job in flight keeps polling.
    let current_page = app.session.page().ok().map(|page| page.0);
    if !app.sync.all_pages && !app.sync.busy && app.sync.page.is_some() && app.sync.page != current_page {
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.page = None;
        app.sync.dirty = false;
        app.sync.error = None;
        app.sync.refresh = app.sync.open;
    }

    poll_diff(app);
    poll_job(app, ctx);
    poll_cancel(app);
    #[cfg(not(target_arch = "wasm32"))]
    if !app.sync.starting {
        let address = app.sync.node.as_ref().map(|node| node.address.clone());
        for event in std::mem::take(&mut app.sync.pushed_jobs) {
            if let Some(address) = address.as_ref() {
                node_event(app, ctx, address, event);
            }
        }
    }
    if app.sync.refresh && !app.sync.busy {
        app.sync.refresh = false;
        if let Ok((snapshot, page)) = snapshot_now(app) {
            start_diff(app, ctx, snapshot, page, workspace.clone());
        }
    }
    if app.sync.open {
        refresh_dirty(app);
    } else {
        app.sync.dirty = false;
    }

    if !app.sync.open {
        return;
    }

    let scope = scope_label(app);
    // Under the header (the macOS toolbar band is taller), at the Sync button's right edge.
    let header_h = if app.ui.mac_inset() { crate::MAC_BAND } else { 44.0 };
    let menu_was_open = app.sync.menu_open;
    let mut out = WidgetOut::default();
    // The anchored area takes its first frame's size from `default_size`; without it the
    // scroll area inherits egui's 400-point default and the widget stops halfway down.
    let max_h = (ctx.content_rect().height() - 140.0).max(220.0);
    // The flight (or failure) card stays above the scroll area so it is visible even when the
    // human had scrolled to the agent and note.
    let job_card = app.sync.job.as_ref().is_some_and(|job| job.in_flight() || job.failed()) || app.sync.blocker().is_some();
    // A fixed, bounded viewport: an anchored Area that grows with its scrollable content would
    // chase its own height, and shrinking to measured content lets egui's clipping hide the rest.
    let widget_h = max_h.min(620.0);
    let scroll_h = (widget_h - 96.0 - if job_card { 86.0 } else { 0.0 }).max(140.0);
    let area = egui::Area::new(Id::new("sync_widget"))
        .anchor(Align2::RIGHT_TOP, vec2(-8.0, header_h + 8.0))
        .default_size(vec2(WIDGET_W, widget_h))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(36) })
                .inner_margin(egui::Margin { left: 19, right: 17, top: 17, bottom: 17 })
                .show(ui, |ui| {
                    ui.set_width(WIDGET_W - 2.0 * PAD);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    out = widget_body(ui, &mut app.sync, &mut app.compose, &scope, scroll_h);
                });
        });

    let escape = ctx.input(|input| input.key_pressed(Key::Escape));
    if escape {
        if menu_was_open {
            app.sync.menu_open = false;
        } else {
            app.sync.open = false;
        }
    }
    let pressed = ctx.input(|input| input.pointer.any_pressed());
    let pointer = ctx.input(|input| input.pointer.interact_pos());
    let inside = pointer.is_some_and(|pos| area.response.rect.contains(pos));
    let just_opened = std::mem::take(&mut app.sync.just_opened);
    if pressed && !inside && !menu_was_open && !just_opened {
        // Clicking a target in the menu is outside the widget; the popup owns that click.
        app.sync.open = false;
        app.sync.menu_open = false;
    }
    if out.close {
        app.sync.open = false;
        app.sync.menu_open = false;
    }
    if let Some(all_pages) = out.scope
        && all_pages != app.sync.all_pages
        && !app.sync.busy
    {
        app.sync.all_pages = all_pages;
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.page = None;
        app.sync.dirty = false;
        app.sync.error = None;
        if let Ok((snapshot, page)) = snapshot_now(app) {
            start_diff(app, ctx, snapshot, page, workspace.clone());
        }
    }
    if let Some(job) = out.cancel {
        start_cancel(app, ctx, job);
    }
    if out.sync && app.sync.open {
        match snapshot_now(app) {
            Ok((snapshot, page)) => start_sync(app, ctx, workspace, snapshot, page),
            Err(message) => app.toast(message),
        }
    }
}

/// The widget's scope line: the current page and how many subpages a page sync takes along.
fn scope_label(app: &FigApp) -> String {
    let Ok(doc) = app.session.doc() else { return String::new() };
    let Ok(page) = app.session.page() else { return String::new() };
    let name = doc.node(page).map(|node| node.name.clone()).unwrap_or_else(|| "this page".to_string());
    match crate::page_tree::descendants(doc, page).len() {
        0 => name,
        1 => format!("{name} + 1 subpage"),
        n => format!("{name} + {n} subpages"),
    }
}

/// What the widget body asked for.
#[derive(Default)]
struct WidgetOut {
    close: bool,
    sync: bool,
    /// The scope the human picked: `Some(true)` for every page, `Some(false)` for this page.
    scope: Option<bool>,
    /// Cancel this open record.
    cancel: Option<String>,
}

/// The widget's frame contents: header, last sync, the diffs, agent, note and action.
fn widget_body(ui: &mut Ui, state: &mut State, compose: &mut crate::compose::State, scope: &str, scroll_h: f32) -> WidgetOut {
    let mut out = WidgetOut::default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        ui.label(egui::RichText::new("SYNC").font(mono(9.5)).color(ACCENT));
        ui.add_space(4.0);
        let enabled = !state.busy;
        if scope_chip(ui, "This page", !state.all_pages, enabled) {
            out.scope = Some(false);
        }
        if scope_chip(ui, "All pages", state.all_pages, enabled) {
            out.scope = Some(true);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if close_button(ui) {
                out.close = true;
            }
        });
    });
    ui.add_space(7.0);
    let covers = if state.all_pages { "every page".to_string() } else { scope.to_string() };
    ui.label(egui::RichText::new(covers).font(regular(11.0)).color(INK));
    ui.add_space(6.0);
    let last_line = match &state.last {
        Some(last) => last_line(last),
        None => "first sync".to_string(),
    };
    ui.label(egui::RichText::new(last_line).font(mono(9.0)).color(MUTED));
    if state.dirty {
        ui.add_space(7.0);
        ui.label(egui::RichText::new("the map changed since this diff").font(mono(8.5)).color(ACCENT));
    }
    let mut carded = false;
    if let Some(job) = state.job.as_ref() {
        if job.in_flight() {
            ui.add_space(7.0);
            flight_card(ui, job, compose);
            carded = true;
        } else if job.failed() {
            let reason = job.reason.clone().unwrap_or_else(|| "The sync failed.".to_string());
            ui.add_space(7.0);
            failed_card(ui, &reason);
            carded = true;
        }
    }
    if let Some(open) = state.blocker() {
        ui.add_space(7.0);
        if open_card(ui, open, &compose.targets, state.cancelling, now_secs()) {
            out.cancel = Some(open.id.clone());
        }
        carded = true;
    }
    ui.add_space(if carded { 9.0 } else { 7.0 });
    divider(ui);
    ui.add_space(7.0);

    // The widget hugs its content, as the design draws it: the viewport takes the height the
    // content measured last frame and scrolls only past the window's height.
    let fit = if state.content_h > 0.0 { state.content_h.min(scroll_h) } else { scroll_h };
    let scrolled =
        egui::ScrollArea::vertical().id_salt("sync_widget_body").max_height(fit).min_scrolled_height(fit).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                if let Some(error) = state.error.clone() {
                    error_block(ui, &error);
                    ui.add_space(10.0);
                }

                match state.diff.as_ref() {
                    Some(diff) => {
                        design_section(ui, &diff.map);
                        ui.add_space(7.0);
                        code_section(ui, state.last.as_ref(), &diff.code);
                    }
                    None => {
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(7.0, 0.0);
                            if state.busy {
                                ui.add(egui::Spinner::new().size(11.0).color(ACCENT));
                                ui.label(egui::RichText::new("Reading the diff…").font(regular(11.0)).color(MUTED));
                            } else {
                                ui.label(egui::RichText::new("No diff read yet.").font(regular(11.0)).color(MUTED));
                            }
                        });
                    }
                }

                ui.add_space(7.0);
                divider(ui);
                ui.add_space(7.0);
                ui.label(egui::RichText::new("AGENT").font(mono(8.5)).color(MUTED));
                ui.add_space(7.0);
                // The composer's agent button and menu: the same agent, chosen the same way.
                if let Some(target) = agent_button(ui, state, compose) {
                    compose.choose(target);
                }

                ui.add_space(7.0);
                ui.label(egui::RichText::new("NOTE FOR THIS SYNC").font(mono(8.5)).color(MUTED));
                ui.add_space(7.0);
                note_field(ui, &mut state.note);

                ui.add_space(7.0);
                let syncing = state.starting || state.in_flight();
                let locked = state.locked();
                let target = compose.target.as_ref();
                let ready = state.diff.as_ref().is_some_and(|diff| diff.ok);
                let enabled = !locked && target.is_some() && ready && state.has_work();
                let label = if syncing {
                    "Syncing…".to_string()
                } else {
                    match target {
                        Some(target) => format!("Sync with {}", kind_label(target.kind())),
                        None => "Sync".to_string(),
                    }
                };
                if action_button(ui, &label, enabled, syncing) {
                    out.sync = true;
                }
                ui.add_space(7.0);
                let hint = if state.blocker().is_some() {
                    "Sync waits until the open record is committed or cancelled."
                } else if target.is_none() {
                    if compose.targets.is_empty() { "No agent is available in this workspace yet." } else { "Choose an agent to sync." }
                } else if syncing {
                    "The map stays editable while the sync runs."
                } else if state.busy {
                    "Reading the diff…"
                } else if !ready {
                    "Refresh the diff to sync."
                } else if !state.has_work() {
                    "Change the map or add a note to sync."
                } else {
                    "The map stays editable."
                };
                ui.label(egui::RichText::new(hint).font(mono(8.5)).color(MUTED));
            },
        );
    let measured = scrolled.content_size.y;
    if (measured - state.content_h).abs() > 0.5 {
        state.content_h = measured;
        ui.ctx().request_repaint();
    }
    out
}

/// The header's ×.
fn close_button(ui: &mut Ui) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    a11y::button(&response, "Close sync");
    let hot = response.hovered();
    if hot {
        ui.painter().circle_filled(rect.center(), 9.0, SOFT);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", mono(11.0), if hot { ACCENT } else { MUTED });
    response.clicked()
}

/// The in-flight card: spinner, `Sync in flight · kind`, and when it started.
fn flight_card(ui: &mut Ui, job: &Job, compose: &crate::compose::State) {
    let kind =
        job.kind().or_else(|| compose.target.as_ref().map(|target| kind_label(target.kind()).to_string())).unwrap_or_else(|| "agent".to_string());
    egui::Frame::NONE
        .fill(SOFT)
        .stroke(Stroke::new(1.0, ACCENT))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new(format!("Sync in flight · {kind}")).font(semibold(11.0)).color(ACCENT));
            let started = job.started.trim();
            let line = if started.is_empty() { "started just now".to_string() } else { format!("started {started}") };
            ui.label(egui::RichText::new(line).font(mono(9.0)).color(MUTED));
        });
}

/// The open record (or another Sync) the node is waiting on: what it is, whose it is, how long it
/// has waited, and a Cancel button for a record. Returns whether Cancel was clicked.
fn open_card(ui: &mut Ui, job: &Job, targets: &[TargetInfo], cancelling: bool, now: Option<u64>) -> bool {
    let mut clicked = false;
    egui::Frame::NONE
        .fill(SOFT)
        .stroke(Stroke::new(1.0, ACCENT))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new(open_title(job)).font(semibold(11.0)).color(ACCENT));
            let line = format!("by {} · {}", job_owner(job, targets), waited(&job.started, now));
            ui.label(egui::RichText::new(line).font(mono(9.0)).color(MUTED));
            if let Some(reason) = job.reason.as_deref().map(str::trim).filter(|reason| !reason.is_empty()) {
                ui.add(egui::Label::new(egui::RichText::new(reason).font(mono(9.0)).color(MUTED)).wrap());
            }
            if job.is_record() {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    let label = if cancelling { "Cancelling…" } else { "Cancel record" };
                    clicked = secondary_button(ui, label, !cancelling);
                    ui.label(egui::RichText::new("commits nothing").font(mono(8.5)).color(FAINT));
                });
            }
        });
    clicked
}

/// `Explanation record open`, `Bug fix record open`, or another `Sync in flight`.
pub(crate) fn open_title(job: &Job) -> &'static str {
    match job.work.trim() {
        "explain" => "Explanation record open",
        "fix" => "Bug fix record open",
        _ => "Sync in flight",
    }
}

/// Whose job it is: the agent in its pane, by the name the agent menu uses.
pub(crate) fn job_owner(job: &Job, targets: &[TargetInfo]) -> String {
    let Some(pane) = job.pane() else { return "an agent outside Herdr".to_string() };
    targets
        .iter()
        .find_map(|info| match &info.target {
            Target::Existing { pane: p, kind, .. } if p == pane => Some(if info.label.trim().is_empty() {
                kind_label(kind).to_string()
            } else {
                format!("{} · {}", info.label.trim(), kind_label(kind))
            }),
            _ => None,
        })
        .unwrap_or_else(|| format!("pane {pane}"))
}

/// `waited 12 min` from the job's start (Unix seconds) to now.
pub(crate) fn waited(started: &str, now: Option<u64>) -> String {
    let (Some(start), Some(now)) = (started.trim().parse::<u64>().ok(), now) else { return "open".to_string() };
    let minutes = now.saturating_sub(start) / 60;
    match minutes {
        0 => "opened just now".to_string(),
        1..=59 => format!("waited {minutes} min"),
        _ => format!("waited {} h {} min", minutes / 60, minutes % 60),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_secs() -> Option<u64> {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

#[cfg(target_arch = "wasm32")]
fn now_secs() -> Option<u64> {
    None
}

/// A quiet outlined button for an action beside the main one.
fn secondary_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(10.0)) + 22.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    a11y::button(&response, label);
    let fill = if enabled && response.hovered() { PANEL } else { SOFT };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, if enabled { ACCENT } else { BORDER }), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(10.0), if enabled { ACCENT } else { FAINT });
    response.clicked() && enabled
}

/// Cancel an open record on a thread; the diff is read again once the node answers.
#[cfg(not(target_arch = "wasm32"))]
fn start_cancel(app: &mut FigApp, ctx: &egui::Context, job: String) {
    if app.sync.cancelling {
        return;
    }
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let request = json!({"id": 1, "method": "sync.cancel", "job": job}).to_string();
    let (tx, rx) = mpsc::channel();
    app.sync.cancel_rx = Some(rx);
    app.sync.cancelling = true;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-cancel".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ask(&node.address, &request, STATUS_TIMEOUT)))
            .unwrap_or_else(|_| Err("The cancel request failed unexpectedly.".to_string()))
            .and_then(|text| {
                let value: Value = serde_json::from_str(&text).map_err(|e| format!("The node sent an unreadable answer: {e}"))?;
                if value["ok"] == true { Ok(()) } else { Err(value["error"].as_str().unwrap_or("The node refused the cancel.").to_string()) }
            });
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.cancel_rx = None;
        app.sync.cancelling = false;
        app.sync.error = Some(format!("Could not cancel the record: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_cancel(app: &mut FigApp, _ctx: &egui::Context, _job: String) {
    app.toast("The node is only available in the desktop app.");
}

/// Take in the cancel thread's answer.
fn poll_cancel(app: &mut FigApp) {
    let received = match app.sync.cancel_rx.as_ref().map(Receiver::try_recv) {
        None | Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Disconnected)) => Err("The cancel request ended without an answer.".to_string()),
    };
    app.sync.cancel_rx = None;
    app.sync.cancelling = false;
    match received {
        Ok(()) => {
            app.sync.error = None;
            app.sync.refresh = true;
            app.toast("Record cancelled: nothing was committed.");
        }
        Err(e) => app.sync.error = Some(e),
    }
}

/// A failed job's reason, kept in the widget so the next sync can be set up.
fn failed_card(ui: &mut Ui, reason: &str) {
    egui::Frame::NONE
        .fill(SOFT)
        .stroke(Stroke::new(1.0, DANGER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new("Sync failed").font(semibold(11.0)).color(DANGER));
            ui.add(egui::Label::new(egui::RichText::new(reason).font(mono(9.0)).color(DANGER)).wrap());
        });
}

/// A request error (a refused diff, a lost status poll).
fn error_block(ui: &mut Ui, error: &str) {
    egui::Frame::NONE
        .fill(SOFT)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            paragraph(ui, error, 10.5, DANGER);
        });
}

/// `DESIGN CHANGES`: the counts, then up to four names of each kind.
fn design_section(ui: &mut Ui, map: &MapDiff) {
    ui.label(egui::RichText::new("DESIGN CHANGES").font(mono(9.0)).color(ACCENT));
    if map.total() == 0 {
        ui.add_space(7.0);
        ui.label(egui::RichText::new("No design changes since the last sync.").font(regular(10.5)).color(MUTED));
        return;
    }
    if !map.added.is_empty() {
        change_group(ui, "+", "added", MAP_INK, map.added.iter().map(|card| card.name.as_str()));
    }
    if !map.changed.is_empty() {
        change_group(ui, "~", "changed", ACCENT, map.changed.iter().map(|change| change.name.as_str()));
    }
    if !map.removed.is_empty() {
        change_group(ui, "−", "removed", DANGER, map.removed.iter().map(|card| card.name.as_str()));
    }
}

/// `+  3 added` in the group's colour, then its names on one muted line (`A · B · C`).
fn change_group<'a>(ui: &mut Ui, marker: &str, label: &str, color: Color32, names: impl Iterator<Item = &'a str>) {
    let names: Vec<&str> = names.collect();
    ui.add_space(7.0);
    ui.label(egui::RichText::new(format!("{marker}  {} {label}", names.len())).font(regular(10.5)).color(color));
    if names.is_empty() {
        return;
    }
    ui.add_space(7.0);
    indented_name(ui, &names_line(&names), MUTED);
}

/// Up to [`NAMES`] names joined by ` · `, then `+N more`.
fn names_line(names: &[&str]) -> String {
    let mut line = names.iter().take(NAMES).copied().collect::<Vec<_>>().join(" · ");
    if names.len() > NAMES {
        line.push_str(&format!(" · +{} more", names.len() - NAMES));
    }
    line
}

/// A line under a count, indented under its label and elided to the widget's width.
fn indented_name(ui: &mut Ui, name: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let room = ui.available_width().max(40.0);
        let shown = widgets::elide_font(ui, name, room, regular(10.5));
        ui.label(egui::RichText::new(shown).font(regular(10.5)).color(color));
    });
}

/// `CODE SINCE <commit>`: file and commit counts, their lists, and the service's note.
fn code_section(ui: &mut Ui, last: Option<&Last>, code: &CodeDiff) {
    let first = last.is_none();
    let since = last
        .map(|last| last.commit.trim().to_string())
        .filter(|commit| !commit.is_empty())
        .or_else(|| code.from.as_deref().map(str::trim).filter(|commit| !commit.is_empty()).map(str::to_string));
    let title = match (first, since) {
        (true, _) => "CODE SINCE FIRST SYNC".to_string(),
        (false, Some(commit)) => format!("CODE SINCE {}", short_commit(&commit)),
        (false, None) => "CODE SINCE THE LAST SYNC".to_string(),
    };
    ui.label(egui::RichText::new(title).font(mono(9.0)).color(MUTED));
    if !code.scope.is_empty() {
        let shown: Vec<&str> = code.scope.iter().take(FILES).map(String::as_str).collect();
        let mut line = format!("in {}", shown.join(", "));
        if code.scope.len() > FILES {
            line.push_str(&format!(" +{} more", code.scope.len() - FILES));
        }
        ui.add_space(7.0);
        ui.add(egui::Label::new(egui::RichText::new(line).font(regular(10.0)).color(FAINT)).wrap());
    }
    ui.add_space(7.0);
    if first {
        ui.label(egui::RichText::new("first sync: the map is the baseline.").font(regular(10.5)).color(MUTED));
    } else {
        let files = code.files.len();
        let commits = code.commits.len();
        if files == 0 && commits == 0 && code.note.as_deref().unwrap_or("").trim().is_empty() {
            ui.label(egui::RichText::new("No code changes since the last sync.").font(regular(10.5)).color(MUTED));
        } else {
            let counts = format!("{files} file{} · {commits} commit{}", if files == 1 { "" } else { "s" }, if commits == 1 { "" } else { "s" });
            ui.label(egui::RichText::new(counts).font(regular(9.5)).color(MUTED));
        }
    }
    for file in code.files.iter().take(FILES) {
        ui.add_space(7.0);
        file_row(ui, file);
    }
    if code.files.len() > FILES {
        ui.add_space(7.0);
        ui.label(egui::RichText::new(format!("+{} more files", code.files.len() - FILES)).font(regular(10.0)).color(FAINT));
    }
    if let Some(note) = code.note.as_deref().map(str::trim).filter(|note| !note.is_empty()) {
        ui.add_space(7.0);
        paragraph(ui, note, 10.0, FAINT);
    }
}

/// One changed file: `service/src/sync.rs   +240/−3`, one line.
fn file_row(ui: &mut Ui, file: &FileDelta) {
    let stats = format!("   +{}/−{}", file.added, file.removed);
    let room = (ui.available_width() - text_width(ui, &stats, regular(10.0))).max(40.0);
    let path = widgets::elide_start_font(ui, &file.path, room, regular(10.0));
    ui.label(egui::RichText::new(format!("{path}{stats}")).font(regular(10.0)).color(INK));
}

/// The agent button and its menu, shared with the composer; the pick, if any.
fn agent_button(ui: &mut Ui, state: &mut State, compose: &crate::compose::State) -> Option<Target> {
    let label = agent_menu::button_label(&compose.targets, compose.target.as_ref(), "Choose an agent");
    let width = agent_menu::button_width(ui, &label, 120.0, ui.available_width().min(280.0));
    let response =
        agent_menu::button(ui, Id::new("sync_agent_button"), &compose.targets, compose.target.as_ref(), &label, vec2(width, 28.0), state.menu_open);
    if response.clicked() {
        state.menu_open = !state.menu_open;
        if state.menu_open {
            state.menu.opened();
        }
    }
    let options =
        agent_menu::Options { loading: compose.loading_targets(), error: compose.targets_error.as_deref(), ..agent_menu::Options::default() };
    agent_menu::popup(
        ui,
        Id::new("sync_targets"),
        &response,
        &mut state.menu_open,
        &mut state.menu,
        &compose.targets,
        compose.target.as_ref(),
        options,
    )
}

/// One scope choice in the widget's header.
fn scope_chip(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(9.5)) + 18.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 19.0), sense);
    let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    a11y::button(&response, &format!("Sync {}", label.to_lowercase()));
    let hot = enabled && response.hovered();
    let fill = if selected {
        Color32::from_rgb(0xF1, 0xEC, 0xE7)
    } else if hot {
        SOFT
    } else {
        PANEL
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    let border = if selected { ACCENT } else { BORDER };
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = if selected || hot { ACCENT } else { MUTED };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(9.5), ink);
    response.clicked() && enabled && !selected
}

/// The note field: multiline, capped, with the design's placeholder.
fn note_field(ui: &mut Ui, note: &mut String) {
    // One line tall until the note needs more, as the design draws it.
    egui::Frame::NONE
        .fill(Color32::from_rgb(0xF1, 0xF0, 0xED))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(11, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::TextEdit::multiline(note)
                    .frame(egui::Frame::NONE)
                    .font(regular(10.5))
                    .desired_width(f32::INFINITY)
                    .desired_rows(1)
                    .hint_text(egui::RichText::new("Add a note for the agent…").font(regular(10.5)).color(FAINT)),
            );
        });
    if note.chars().count() > MAX_NOTE_CHARS {
        *note = note.chars().take(MAX_NOTE_CHARS).collect();
    }
}

/// The widget's one action: accent when it can run, soft while it cannot.
fn action_button(ui: &mut Ui, label: &str, enabled: bool, busy: bool) -> bool {
    // As wide as its label with 16 pt either side, 26 pt tall.
    let width = text_width(ui, label, semibold(10.5)) + 32.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    a11y::button(&response, label);
    let fill = if enabled { if response.hovered() { ACCENT_HOVER } else { ACCENT } } else { Color32::from_rgb(0xE8, 0xE5, 0xE0) };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    let ink = if enabled { Color32::WHITE } else { FAINT };
    let _ = busy;
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(10.5), ink);
    response.clicked() && enabled
}

/// Read the current document's snapshot and the page it is limited to (`None` for every page). The
/// doc and its current page must both exist.
fn snapshot_now(app: &FigApp) -> Result<(Snapshot, Option<u64>), String> {
    let doc = app.session.doc().map_err(|_| "Open a map to sync.".to_string())?;
    let page = app.session.page().map_err(|_| "This map has no page to sync.".to_string())?;
    if doc.node(page).is_none() {
        return Err("This map's page is not available to sync.".to_string());
    }
    let scope = (!app.sync.all_pages).then_some(page);
    let snapshot = Snapshot::of_pages(doc, scope);
    if snapshot.pages.is_empty() {
        return Err("This map has no pages to sync.".to_string());
    }
    Ok((snapshot, scope.map(|page| page.0)))
}

/// Rebuild the current snapshot and compare it with the one the diff was read from.
fn refresh_dirty(app: &mut FigApp) {
    let scope = app.sync.page.map(NodeId);
    let current = app.session.doc().ok().map(|doc| Snapshot::of_pages(doc, scope));
    let dirty = match (app.sync.snapshot.as_ref(), current.as_ref()) {
        (Some(baseline), Some(current)) => current.differs(baseline),
        (None, _) => false,
        (Some(_), None) => true,
    };
    app.sync.dirty = dirty;
}

/// Ask for the diff of `snapshot`, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start_diff(app: &mut FigApp, ctx: &egui::Context, snapshot: Snapshot, page: Option<u64>, workspace: String) {
    if app.sync.busy {
        return;
    }
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let request = json!({"id": 1, "method": "diff", "workspace": workspace, "session": node.session, "page": page}).to_string();
    // Job events (a record opening or ending) arrive for the node the diff came from.
    if !app.sync.starting && !app.sync.in_flight() {
        app.sync.node = Some(node.clone());
    }
    let (tx, rx) = mpsc::channel();
    app.sync.rx = Some(rx);
    app.sync.busy = true;
    app.sync.error = None;
    app.sync.snapshot = Some(snapshot);
    app.sync.page = page;
    app.sync.dirty = false;
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-diff".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| diff_request(&address, &request)))
            .unwrap_or_else(|_| Err("The diff request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.rx = None;
        app.sync.busy = false;
        app.sync.error = Some(format!("Could not ask for the diff: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_diff(app: &mut FigApp, _ctx: &egui::Context, _snapshot: Snapshot, _page: Option<u64>, _workspace: String) {
    app.sync.busy = false;
    app.sync.error = Some("The node is only available in the desktop app.".to_string());
}

/// Ask the service to sync the map with the chosen target.
#[cfg(not(target_arch = "wasm32"))]
fn start_sync(app: &mut FigApp, ctx: &egui::Context, workspace: String, _snapshot: Snapshot, page: Option<u64>) {
    if app.sync.busy || app.sync.starting || app.sync.in_flight() {
        return;
    }
    let Some(target) = app.compose.target.clone() else {
        app.toast("Choose an agent to sync with.");
        return;
    };
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let note = app.sync.note.chars().take(MAX_NOTE_CHARS).collect::<String>();
    app.sync.node = Some(node.clone());
    let request = json!({
        "id": 1,
        "method": "sync",
        "workspace": workspace,
        "target": target.line(),
        "prompt": note,
        "session": node.session,
        "page": page,
    })
    .to_string();
    let (tx, rx) = mpsc::channel();
    app.sync.job_rx = Some(rx);
    app.sync.busy = true;
    app.sync.starting = true;
    app.sync.error = None;
    app.sync.applied = app.sync.diff.as_ref().map(|diff| diff.map.total()).unwrap_or(0);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-start".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_request(&address, &request)))
            .unwrap_or_else(|_| Err("The sync request failed unexpectedly.".to_string()));
        let _ = tx.send(JobMsg::Started(reply));
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.job_rx = None;
        app.sync.busy = false;
        app.sync.starting = false;
        app.sync.error = Some(format!("Could not start the sync: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_sync(app: &mut FigApp, _ctx: &egui::Context, _workspace: String, _snapshot: Snapshot, _page: Option<u64>) {
    app.sync.busy = false;
    app.sync.starting = false;
    app.toast("The node is only available in the desktop app.");
}

/// Poll a job's status once a second on a named thread until it settles, however long the agent
/// works: a sync has no time limit. The thread ends when the widget stops listening.
#[cfg(not(target_arch = "wasm32"))]
fn start_poll(app: &mut FigApp, ctx: &egui::Context, job_id: String) {
    let (tx, rx) = mpsc::channel();
    app.sync.job_rx = Some(rx);
    let Some(node) = app.sync.node.clone() else { return };
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-status".into()).spawn(move || {
        let mut misses = 0u8;
        loop {
            std::thread::sleep(POLL_EVERY);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| status_request(&address, &job_id, &node.session))) {
                Ok(Ok(reply)) => {
                    let settled = !reply.ok || reply.job.as_ref().is_none_or(|job| !job.in_flight());
                    if tx.send(JobMsg::Status(Ok(reply))).is_err() {
                        return;
                    }
                    context.request_repaint();
                    if settled {
                        return;
                    }
                    misses = 0;
                }
                Ok(Err(e)) => {
                    misses = misses.saturating_add(1);
                    if misses >= POLL_MISSES {
                        let _ = tx.send(JobMsg::Status(Err(e)));
                        context.request_repaint();
                        return;
                    }
                }
                Err(_) => {
                    let _ = tx.send(JobMsg::Status(Err("The sync status request failed unexpectedly.".to_string())));
                    context.request_repaint();
                    return;
                }
            }
        }
    });
    if let Err(e) = spawned {
        app.sync.job_rx = None;
        app.sync.error = Some(format!("Could not poll the sync: {e}"));
        if let Some(job) = app.sync.job.as_mut() {
            job.state = "failed".to_string();
            job.reason = Some(format!("Could not poll the sync: {e}"));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_poll(app: &mut FigApp, _ctx: &egui::Context, _job_id: String) {
    app.sync.job_rx = None;
}

/// Take in the diff thread's answer.
fn poll_diff(app: &mut FigApp) {
    let received = app.sync.rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.sync.rx = None;
            app.sync.busy = false;
            app.sync.error = Some("The diff request ended without an answer.".to_string());
            return;
        }
    };
    app.sync.rx = None;
    app.sync.busy = false;
    match received {
        Ok(reply) if reply.ok => {
            app.sync.last = reply.last.clone();
            app.sync.diff = Some(reply);
            app.sync.error = None;
            app.sync.dirty = false;
        }
        Ok(reply) => {
            app.sync.error = Some(reply.error.unwrap_or_else(|| "The node refused the diff.".to_string()));
        }
        Err(e) => app.sync.error = Some(e),
    }
}

/// Take in the sync-start or status thread's next answer.
fn poll_job(app: &mut FigApp, ctx: &egui::Context) {
    let received = app.sync.job_rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(message)) => message,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.sync.job_rx = None;
            app.sync.busy = false;
            app.sync.starting = false;
            fail_flight(app, "The sync status request ended without an answer.".to_string());
            return;
        }
    };
    apply_job(app, ctx, received);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn node_event(app: &mut FigApp, ctx: &egui::Context, address: &str, mut event: Value) {
    if app.sync.node.as_ref().is_none_or(|node| node.address != address) {
        return;
    }
    if app.sync.starting {
        if app.sync.pushed_jobs.len() < 32 {
            app.sync.pushed_jobs.push(event);
        }
        return;
    }
    if app.sync.job.as_ref().is_none_or(|job| event["job"]["id"].as_str() != Some(job.id.as_str()) || !job.in_flight()) {
        // Another job: a record opened or ended, or the one blocking Sync changed. Re-read.
        let blocking = app.sync.blocker().is_some_and(|open| event["job"]["id"].as_str() == Some(open.id.as_str()));
        let record = matches!(event["job"]["kind"].as_str(), Some("explain" | "fix"));
        if blocking || record {
            app.sync.refresh = true;
            ctx.request_repaint();
        }
        return;
    }
    event["ok"] = Value::Bool(true);
    match serde_json::from_value::<StatusReply>(event) {
        Ok(reply) => apply_job(app, ctx, JobMsg::Status(Ok(reply))),
        Err(_) => {
            app.sync.error = Some("The node sent an invalid job update.".into());
        }
    }
}

fn apply_job(app: &mut FigApp, ctx: &egui::Context, received: JobMsg) {
    match received {
        JobMsg::Started(result) => {
            app.sync.busy = false;
            app.sync.starting = false;
            match result {
                Ok(reply) if reply.ok => match reply.job.filter(|job| !job.id.trim().is_empty()) {
                    Some(job) => {
                        app.sync.error = None;
                        app.sync.note.clear();
                        app.sync.job = Some(job.clone());
                        start_poll(app, ctx, job.id);
                    }
                    None => app.sync.error = Some("The node started no sync job.".to_string()),
                },
                Ok(reply) => {
                    app.sync.error = Some(reply.error.unwrap_or_else(|| "The node refused the sync.".to_string()));
                    // A refusal is often a record or Sync already running: show it.
                    app.sync.refresh = true;
                }
                Err(e) => app.sync.error = Some(e),
            }
        }
        JobMsg::Status(result) => match result {
            Ok(reply) if reply.ok => {
                let Some(job) = reply.job else {
                    app.sync.job_rx = None;
                    fail_flight(app, "The node sent a status with no job.".to_string());
                    return;
                };
                if let Some(last) = reply.last {
                    app.sync.last = Some(last);
                }
                let state = job.state.trim().to_ascii_lowercase();
                if job.in_flight() {
                    app.sync.job = Some(job);
                    return;
                }
                app.sync.job_rx = None;
                app.sync.job = Some(job);
                match state.as_str() {
                    "done" => {
                        // The applied changes are gone from the new baseline: start the next diff
                        // from the map as it is now.
                        app.sync.diff = None;
                        app.sync.snapshot = None;
                        app.sync.refresh = true;
                        app.sync.error = None;
                        let changes = app.sync.applied;
                        app.toast(if changes == 0 {
                            "Synced".to_string()
                        } else {
                            format!("Synced · {changes} change{}", if changes == 1 { "" } else { "s" })
                        });
                    }
                    "failed" => {
                        let reason = app.sync.job.as_ref().and_then(|job| job.reason.clone()).unwrap_or_else(|| "The sync failed.".to_string());
                        app.toast(format!("Sync failed: {reason}"));
                    }
                    other => {
                        let message = if other.is_empty() {
                            "The node sent a sync status this app can't read.".to_string()
                        } else {
                            format!("The node sent an unknown sync state ({other}).")
                        };
                        app.sync.error = Some(message);
                    }
                }
            }
            Ok(reply) => {
                app.sync.job_rx = None;
                let message = reply.error.unwrap_or_else(|| "The node refused the sync status.".to_string());
                fail_flight(app, message);
            }
            Err(e) => {
                app.sync.job_rx = None;
                fail_flight(app, e);
            }
        },
    }
}

/// Stop showing a flight as running when its status channel is gone.
fn fail_flight(app: &mut FigApp, reason: String) {
    let mut marked = false;
    if let Some(job) = app.sync.job.as_mut()
        && job.in_flight()
    {
        job.state = "failed".to_string();
        job.reason = Some(reason.clone());
        marked = true;
    }
    if !marked {
        app.sync.error = Some(reason);
    }
}

/// The `diff` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn diff_request(address: &str, request: &str) -> Result<DiffReply, String> {
    let line = ask(address, request, DIFF_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a diff this app can't read: {e}"))
}

/// The `sync` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn sync_request(address: &str, request: &str) -> Result<SyncReply, String> {
    let line = ask(address, request, SYNC_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a sync this app can't read: {e}"))
}

/// The `sync_status` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn status_request(address: &str, job: &str, session: &str) -> Result<StatusReply, String> {
    let request = json!({"id": 1, "method": "sync_status", "session":session, "job": job}).to_string();
    let line = ask(address, &request, STATUS_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a sync status this app can't read: {e}"))
}

/// Ask the node one newline-terminated JSON request and return its reply line.
#[cfg(not(target_arch = "wasm32"))]
fn ask(address: &str, request: &str, timeout: Duration) -> Result<String, String> {
    crate::node::ask(address, request, timeout)
}

/// `v<short commit> · <at>`, or what the service did say.
fn last_line(last: &Last) -> String {
    let commit = short_commit(&last.commit);
    let at = last.at.trim();
    match (commit.is_empty(), at.is_empty()) {
        (false, false) => format!("v{commit} · {at}"),
        (false, true) => format!("v{commit}"),
        (true, false) => format!("synced {at}"),
        (true, true) => "synced".to_string(),
    }
}

/// The first seven characters of a commit hash.
fn short_commit(commit: &str) -> String {
    commit.trim().chars().take(7).collect()
}

fn kind_label(kind: &str) -> &str {
    let kind = kind.trim();
    if kind.is_empty() { "agent" } else { kind }
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, BORDER));
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}
