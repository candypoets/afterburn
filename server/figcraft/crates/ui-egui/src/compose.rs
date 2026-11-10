//! The bottom comment composer: one bar that is both the comment box and the prompt box.
//!
//! The selection tells it what a comment is about. Enter saves the comment; Cmd+Enter sends the
//! input as the prompt with every comment in scope attached. Shift+Enter adds a line. Clicking a
//! chip opens its comment back in the input to edit; a second click (or Esc) goes back to a new
//! comment, and only the chip's × removes the
//! comment. The service is reached over plain TCP on a thread of its own, so the window never
//! waits on the network.
//!
//! The recipient is the shared agent button and menu ([`crate::agent_menu`]). It defaults to the
//! page's agent, else the first idle agent in stable workspace order; with none idle, nobody. Each
//! page remembers its agent: switching pages selects that page's agent again; a page with no
//! history uses the last agent used anywhere. Once shown, a status change never moves it.
//!
//! Map mode's right-click menu lives here too: the human gets Copy and Comment, never the
//! editor's tools.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};

use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Margin, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::agent_menu::{self, kind_label, recipient};
use crate::theme::regular;
use crate::widgets;
#[cfg(not(target_arch = "wasm32"))]
use crate::workspaces;
use crate::workspaces::Target;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// The height of the row under the field: the recipient chip and Send.
const ROW_H: f32 = 32.0;
/// Send's least width; `Review send` widens it.
const SEND_W: f32 = 84.0;
/// The recipient chip's width range; a long agent name is elided inside it.
const CHIP_MIN_W: f32 = 120.0;
const CHIP_MAX_W: f32 = 280.0;
/// How many draft chips fit in the strip before `+N`.
const VISIBLE_DRAFTS: usize = 3;

/// The warm map palette: a white paper bar like the map bar above it.
#[allow(non_snake_case)]
fn PANEL() -> Color32 {
    crate::map_colors::get().panel
}
#[allow(non_snake_case)]
fn SOFT() -> Color32 {
    crate::map_colors::get().soft
}
#[allow(non_snake_case)]
fn BORDER() -> Color32 {
    crate::map_colors::get().border
}
#[allow(non_snake_case)]
fn INK() -> Color32 {
    crate::map_colors::get().ink
}
#[allow(non_snake_case)]
fn MUTED() -> Color32 {
    crate::map_colors::get().muted
}
#[allow(non_snake_case)]
fn FAINT() -> Color32 {
    crate::map_colors::get().faint
}
#[allow(non_snake_case)]
fn ACCENT() -> Color32 {
    crate::map_colors::get().accent
}
#[allow(non_snake_case)]
fn ACCENT_HOVER() -> Color32 {
    crate::map_colors::get().accent_hover
}
#[allow(non_snake_case)]
fn MAP_INK() -> Color32 {
    crate::map_colors::get().map_ink
}
/// The pale accent behind a comment saved on several cards, and the `N SELECTED` chip.
#[allow(non_snake_case)]
fn ACCENT_TINT() -> Color32 {
    crate::map_colors::get().accent_tint
}
#[allow(non_snake_case)]
fn ACCENT_TINT_HOVER() -> Color32 {
    crate::map_colors::get().accent_tint_hover
}
#[allow(non_snake_case)]
fn DANGER() -> Color32 {
    crate::map_colors::get().danger
}
/// The draft open in the field, and that field: a blue that leaves the warm accents alone.
#[allow(non_snake_case)]
fn EDIT() -> Color32 {
    crate::map_colors::get().edit
}
#[allow(non_snake_case)]
fn EDIT_FILL() -> Color32 {
    crate::map_colors::get().edit_fill
}
/// A working agent's ink, and the help line's when nobody is chosen.
#[allow(non_snake_case)]
fn WORKING() -> Color32 {
    crate::map_colors::get().working_ink
}
/// Send while it can't send.
#[allow(non_snake_case)]
fn DISABLED_FILL() -> Color32 {
    crate::map_colors::get().disabled_fill
}

/// The help line when no agent is idle, so the composer chose nobody.
const NO_IDLE: &str = "No idle agent. Choose deliberately or start a new agent.";
/// Why a send stayed in the field: the chosen pane is gone or its state can't be read.
const UNAVAILABLE: &str = "Choose an available agent";
/// The ×'s hit area at the right end of a draft chip.
const DRAFT_X_W: f32 = 22.0;

#[cfg(not(target_arch = "wasm32"))]
const TARGETS_TIMEOUT: Duration = Duration::from_secs(10);
/// An agent may have to start (and accept a first-run prompt) before the send returns.
#[cfg(not(target_arch = "wasm32"))]
const SEND_TIMEOUT: Duration = Duration::from_secs(120);
/// A comment kept on the bar until a prompt carries it to an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    /// The card's name when the draft was written.
    pub node: Option<String>,
    /// The page's name when the draft was written.
    pub page: Option<String>,
    /// The card the comment is attached to; `None` for the whole page.
    pub node_id: Option<NodeId>,
    /// The page it belongs to.
    pub page_id: Option<NodeId>,
    /// One comment saved on several cards at once: its drafts share this id and show as one chip.
    pub group: Option<u64>,
}

/// One choice in the target chip: an agent already in the workspace, or a CLI to start.
#[derive(Clone, Debug)]
pub struct TargetInfo {
    pub target: Target,
    /// What the human calls the pane: Herdr's agent name, else the terminal title, else `Pane N`.
    pub label: String,
    /// The pane's terminal title: the task the agent names for itself.
    pub title: Option<String>,
    /// The agent's lifecycle state (`idle`, `working`, …); empty for a new CLI.
    pub status: String,
    /// Focused in the user's Herdr UI.
    pub focused: bool,
    /// A new agent rather than one already in the workspace.
    pub new_agent: bool,
}

/// The service's answer to a `send`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// The agent the service used (`kind`, `name`, `pane`, …).
    #[serde(default)]
    pub agent: Option<Value>,
    /// How many comments the agent received.
    #[serde(default)]
    pub sent: u64,
}

/// The composer's state.
#[derive(Default)]
pub struct State {
    /// The one line being typed.
    pub input: String,
    /// Comments waiting to be sent; they survive selection changes.
    pub drafts: Vec<Draft>,
    /// The draft the field is editing (an index into [`drafts`](Self::drafts)); `None` for a new
    /// comment. Its chip stays in the strip and wears the editing outline.
    pub editing: Option<usize>,
    /// The drafts the in-flight send carries; they leave the bar when it succeeds.
    pub sending: Vec<Draft>,
    /// The agent the send addresses.
    pub target: Option<Target>,
    /// The target menu is open.
    pub menu_open: bool,
    /// The agents in the workspace and the CLIs a new agent can start.
    pub targets: Vec<TargetInfo>,
    /// A send is on its thread.
    pub busy: bool,
    /// The send's answer.
    pub rx: Option<Receiver<Result<SendReply, String>>>,
    /// The selection the input was last cleared for.
    pub last_selection: Vec<NodeId>,
    /// The page the input was last cleared for.
    pub last_page: Option<NodeId>,
    /// Ask the input for the keyboard on the next draw (the context menu's Comment).
    pub focus_input: bool,
    /// The composer's + Page action asked for a page derived from the selected card.
    page_from_card: bool,
    /// The workspace whose targets were asked for (or are being asked for).
    targets_for: Option<String>,
    /// When the agent list was last requested, for the periodic status refresh.
    pub(crate) targets_at: f64,
    /// The target list's answer while it is in flight.
    targets_rx: Option<Receiver<Result<Vec<TargetInfo>, String>>>,
    /// Why the target list is empty, when the service could not answer.
    pub(crate) targets_error: Option<String>,
    /// The agent button (or ⌘⇧A) asked for the Agents modal; it opens on the next frame.
    pub(crate) wants_agents: bool,
    /// The agent each page last used or picked, by page.
    page_agents: HashMap<NodeId, Target>,
    /// The agent each page was last sent a prompt to (its `target` line): while that agent is
    /// working, the page tree shows it designing the page.
    pub(crate) prompted: HashMap<NodeId, String>,
    /// The agent last used or picked on any page: a page with no history starts on it.
    last_used: Option<Target>,
    /// The page the chosen agent belongs to; another page selects its own agent again.
    agent_page: Option<NodeId>,
    /// The page and agent of the send in flight; a new agent's pane replaces its CLI once known.
    sent_to: Option<(Option<NodeId>, Target)>,
    /// Send met a working or blocked agent: the confirmation is open and nothing is sent yet.
    pub(crate) confirm: Option<Confirm>,
    /// Why the last Send did not leave (`Choose an available agent`), until the next choice.
    notice: Option<String>,
    /// The whole page's drafts are shown one by one instead of as one `Whole page` chip.
    page_open: bool,
    /// The last group id given to a comment saved on several cards.
    last_group: u64,
}

/// The send confirmation for a busy recipient; it reads the pane's status live while open.
#[derive(Clone, Debug)]
pub(crate) struct Confirm {
    /// The pane the prompt would go to; fixed while the confirmation is open.
    target: Target,
    /// What would travel: `1 prompt + 2 drafts · Canvas and layers`.
    scope: String,
    /// Cancel, the default, has been given the keyboard.
    focused: bool,
}

/// How ready a recipient is at the moment of sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Idle (or done): the prompt goes straight away.
    Ready,
    /// In the middle of a task: confirm first.
    Working,
    /// Waiting on an answer or an approval: confirm first.
    Blocked,
    /// Gone, disconnected, or a state that can't be read: never send.
    Unavailable,
}

/// What Send (or Cmd+Enter) does with the chosen recipient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendCheck {
    Send,
    Confirm(Readiness),
    Unavailable,
}

impl State {
    /// Open `drafts[index]` in the field with the caret at its end; a second click on the chip it
    /// belongs to goes back to a new comment instead.
    fn toggle_editing(&mut self, index: usize, ctx: &egui::Context) {
        if self.editing == Some(index) {
            self.stop_editing();
            return;
        }
        let Some(text) = self.drafts.get(index).map(|draft| draft.text.clone()) else { return };
        self.input = text;
        self.editing = Some(index);
        self.focus_input = true;
        let end = self.input.chars().count();
        let mut edit = egui::TextEdit::load_state(ctx, input_id()).unwrap_or_default();
        edit.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(end))));
        egui::TextEdit::store_state(ctx, input_id(), edit);
    }

    /// Back to a new comment; the draft keeps whatever it had.
    pub(crate) fn stop_editing(&mut self) {
        if self.editing.take().is_some() {
            self.input.clear();
            self.focus_input = true;
        }
    }

    /// Save the field's text into the draft being edited, then go back to a new comment.
    fn save_edited(&mut self) {
        let Some(index) = self.editing.take() else { return };
        let text = self.input.trim().to_string();
        if !text.is_empty() {
            for member in self.group_of(index) {
                self.drafts[member].text = text.clone();
            }
        }
        self.input.clear();
    }

    /// Keep a comment in its card/page context without asking an agent to do anything.
    fn save_comment(&mut self, names: &Names<'_>) {
        if self.input.trim().is_empty() {
            return;
        }
        if self.editing.is_some() {
            self.save_edited();
        } else if names.cards.len() > 1 {
            // Several cards selected: the same comment on each, kept together as one group.
            self.last_group += 1;
            for (id, name) in names.cards {
                self.drafts.push(Draft {
                    text: self.input.trim().to_string(),
                    node: name.clone(),
                    page: names.page.map(str::to_owned),
                    node_id: Some(*id),
                    page_id: names.page_id,
                    group: Some(self.last_group),
                });
            }
            self.input.clear();
        } else {
            self.drafts.push(Draft {
                text: self.input.trim().to_string(),
                node: names.node.map(str::to_owned),
                page: names.page.map(str::to_owned),
                node_id: names.node_id,
                page_id: names.page_id,
                group: None,
            });
            self.input.clear();
        }
        self.focus_input = true;
    }

    /// The chosen recipient as the latest agent list has it; `None` when nobody is chosen or the
    /// pane has left the list. Panes are matched by id, never by status.
    pub(crate) fn chosen(&self) -> Option<&TargetInfo> {
        let line = self.target.as_ref()?.line();
        self.targets.iter().find(|info| info.target.line() == line)
    }

    /// The rule Send follows, checked again at the moment of sending: an idle agent (or a new one)
    /// takes the prompt; a working or blocked one asks first; anything else never sends.
    fn send_check(&self) -> SendCheck {
        match &self.target {
            None => SendCheck::Unavailable,
            Some(Target::New { .. }) => SendCheck::Send,
            Some(Target::Existing { .. }) => match self.chosen().map(|info| readiness(&info.status)) {
                Some(Readiness::Ready) => SendCheck::Send,
                Some(busy @ (Readiness::Working | Readiness::Blocked)) => SendCheck::Confirm(busy),
                Some(Readiness::Unavailable) | None => SendCheck::Unavailable,
            },
        }
    }

    /// Send or Cmd+Enter was pressed: true when the prompt may leave now. Otherwise the
    /// confirmation opens (`scope` is what it says would travel) or the bar says why not; the
    /// text and drafts stay either way.
    fn request_send(&mut self, scope: String) -> bool {
        self.notice = None;
        match self.send_check() {
            SendCheck::Send => true,
            SendCheck::Confirm(_) => {
                if let Some(target) = self.target.clone() {
                    self.confirm = Some(Confirm { target, scope, focused: false });
                }
                false
            }
            SendCheck::Unavailable => {
                self.notice = Some(UNAVAILABLE.to_string());
                false
            }
        }
    }

    /// The human picked a recipient (in the agent menu, the Sync panel or the Agents list): the
    /// page remembers it, and so does a page with no history of its own.
    pub(crate) fn choose(&mut self, target: Target) {
        self.remember(self.agent_page, target.clone());
        self.target = Some(target);
        self.notice = None;
    }

    /// A prompt about `page` went to `agent` (the node's reply), sent as `sent`: a new CLI's pane
    /// once the node names it.
    pub(crate) fn prompted_on(&mut self, page: NodeId, sent: &Target, agent: Option<&Value>) {
        let pane = agent.and_then(|agent| agent.get("pane")).and_then(Value::as_str).map(str::trim).filter(|pane| !pane.is_empty());
        let line = pane.map_or_else(|| sent.line(), |pane| format!("pane:{pane}"));
        self.prompted.insert(page, line);
    }

    /// The agent designing `page` now: the one last prompted about it, while it is working. Its
    /// kind as the tree names it (`Claude`).
    pub(crate) fn designing(&self, page: NodeId) -> Option<String> {
        let line = self.prompted.get(&page)?;
        let info = self.targets.iter().find(|info| &info.target.line() == line)?;
        (readiness(&info.status) == Readiness::Working).then(|| agent_menu::kind_title(info.target.kind()))
    }

    /// `target` was used or picked on `page`.
    fn remember(&mut self, page: Option<NodeId>, target: Target) {
        if let Some(page) = page {
            self.page_agents.insert(page, target.clone());
        }
        self.last_used = Some(target);
    }

    /// The agent a page starts on: the one it last used, else the last one used anywhere, else
    /// the first idle agent in the list's stable order; with none idle, nobody. A remembered agent
    /// counts only while the list still has it.
    fn page_default(&self, page: Option<NodeId>) -> Option<Target> {
        let listed = |target: &Target| {
            let line = target.line();
            self.targets.iter().find(|info| info.target.line() == line).map(|info| info.target.clone())
        };
        page.and_then(|page| self.page_agents.get(&page))
            .and_then(listed)
            .or_else(|| self.last_used.as_ref().and_then(listed))
            .or_else(|| default_target(&self.targets))
    }

    /// The map moved to `page`: select that page's agent again. Only a page switch changes the
    /// recipient this way; a status change never does.
    fn switch_page(&mut self, page: Option<NodeId>) {
        if self.agent_page == page {
            return;
        }
        self.agent_page = page;
        // Before the list arrives nobody is chosen; the list then takes the page's agent.
        if !self.targets.is_empty() {
            self.target = self.page_default(page);
            self.notice = None;
        }
    }

    /// A send to `sent` from `page` went to `agent` (the node's reply). A new CLI's agent now has a
    /// pane: the page remembers that pane, and the button moves to it if it still shows the CLI,
    /// so the next send does not start another agent.
    fn started(&mut self, page: Option<NodeId>, sent: &Target, agent: &Value) {
        let Target::New { kind } = sent else { return };
        let Some(pane) = agent.get("pane").and_then(Value::as_str).map(str::trim).filter(|pane| !pane.is_empty()) else { return };
        let kind = agent.get("kind").and_then(Value::as_str).map(str::trim).filter(|kind| !kind.is_empty()).unwrap_or(kind);
        let pane = Target::Existing { pane: pane.to_string(), kind: kind.to_string(), status: "working".to_string() };
        if self.last_used.as_ref() == Some(sent) {
            self.last_used = Some(pane.clone());
        }
        if let Some(page) = page {
            self.page_agents.insert(page, pane.clone());
        }
        if self.target.as_ref() == Some(sent) {
            self.target = Some(pane);
        }
    }

    /// Open the Agents modal, where an agent is chosen or a new one is made.
    pub(crate) fn open_menu(&mut self) {
        self.wants_agents = true;
    }

    /// The drafts saved together with `drafts[index]` (itself included), in order.
    fn group_of(&self, index: usize) -> Vec<usize> {
        let Some(draft) = self.drafts.get(index) else { return Vec::new() };
        match draft.group {
            Some(group) => (0..self.drafts.len()).filter(|at| self.drafts[*at].group == Some(group)).collect(),
            None => vec![index],
        }
    }

    /// Remove one draft (and the drafts saved with it), keeping [`editing`](Self::editing) on the
    /// draft it was on.
    fn remove_draft(&mut self, index: usize) {
        let members = self.group_of(index);
        for &member in members.iter().rev() {
            self.drafts.remove(member);
        }
        match self.editing {
            Some(active) if members.contains(&active) => self.stop_editing(),
            Some(active) => self.editing = Some(active - members.iter().filter(|member| **member < active).count()),
            None => {}
        }
    }
}

/// Draw the composer at the bottom of an open map.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    poll_send(app);
    poll_targets(app);
    // The agents belong to the workspace: ask for them when the map first shows.
    if app.active_workspace.is_some() {
        ensure_targets(app, ctx);
    }

    // Selecting another card (or another page) empties the input; the drafts stay above it.
    let selection = app.session.selection();
    let page = app.session.page().ok();
    app.compose.switch_page(page);
    if selection != app.compose.last_selection || page != app.compose.last_page {
        app.compose.input.clear();
        app.compose.editing = None;
        app.compose.page_open = false;
        app.compose.last_selection = selection.clone();
        app.compose.last_page = page;
    }

    let node_id = selection.first().copied();
    let node = node_id.and_then(|id| node_name(app, id));
    let page_name = page.and_then(|id| node_name(app, id));
    let has_selection = !selection.is_empty();
    // Several cards selected: one comment goes on each of them.
    let cards: Vec<(NodeId, Option<String>)> =
        if selection.len() > 1 { selection.iter().map(|id| (*id, node_name(app, *id))).collect() } else { Vec::new() };
    // What a send from here carries: the drafts in the cards' subtrees, or on the whole page.
    let included = included_drafts(app, &selection, page);
    let comments = included.len();
    let plural = if comments == 1 { "" } else { "s" };
    let context = match (node.as_deref(), page_name.as_deref()) {
        _ if cards.len() > 1 => many_cards_context(cards.len()),
        (Some(node), _) => format!("COMMENTING ON · {node} · {comments} comment{plural} in the tree"),
        (None, Some(page)) => format!("COMMENTING ON · {page} · whole page · {comments} comment{plural}"),
        _ => "COMMENTING ON".to_string(),
    };
    let active = app.active_workspace.is_some() && app.session.live().status == figcraft_engine::live::Status::Live;
    if app.session.ds().is_ok_and(|ds| ds.view.comments) {
        draft_marks(app, ctx, page, &selection);
    }
    // The sidebar takes the left edge; the composer belongs to the canvas column.
    // Use this frame's allocated canvas column, including both sidebars. A toggle can change
    // state during drawing; the composer must still match the panels allocated for this frame.
    let (pane, width) = if app.canvas.rect.is_finite() && app.canvas.rect.width() > 0.0 {
        (app.canvas.rect.left() - ctx.content_rect().left(), app.canvas.rect.width())
    } else {
        // A standalone composer can be drawn before the canvas has allocated its first frame.
        let (left, right) = crate::map_panes::widths(app, ctx.content_rect().width());
        (left, (ctx.content_rect().width() - left - right).max(1.0))
    };

    let mut send = false;
    let mut clear = false;
    let enabled = !crate::map_actions::blocking_overlay(app, ctx);
    let composer = egui::Area::new(Id::new("map_composer"))
        .anchor(Align2::LEFT_BOTTOM, vec2(pane, -crate::status_bar::HEIGHT))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let dock = egui::Frame::NONE.fill(PANEL()).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                if !enabled {
                    ui.disable();
                }
                ui.set_width((width - 32.0).max(40.0));
                ui.spacing_mut().item_spacing.y = 0.0;
                let names = Names { node: node.as_deref(), page: page_name.as_deref(), node_id, page_id: page, cards: &cards };
                let out = body(ui, &mut app.compose, &context, &names, has_selection, active, &included, comments);
                send = out.send;
                clear = out.clear;
            });
            // The dock's only edge: one rule along its top.
            let top = dock.response.rect;
            ui.painter().hline(top.x_range(), top.top(), Stroke::new(1.0, BORDER()));
        });
    crate::a11y::area(&mut app.ui.nav, crate::a11y::Region::Properties, composer.response.rect);
    if enabled && !has_selection && comments > 0 && page_comment_chip(ctx, composer.response.rect, comments) {
        app.compose.page_open = true;
        app.compose.focus_input = true;
    }
    crate::a11y::prefer(&mut app.ui.nav, crate::a11y::Region::Properties, input_id());
    if clear {
        app.run("select.none", serde_json::json!({}));
    }
    if app.compose.page_from_card {
        app.compose.page_from_card = false;
        if let (Some(node_id), Some(page_id), Some(node_name), Some(page_name)) = (node_id, page, node.as_deref(), page_name.as_deref()) {
            let number = app.session.doc().map(|doc| doc.pages.len() + 1).unwrap_or(1);
            app.new_page.start_component(
                number,
                crate::new_page::Source {
                    page_id: page_id.0,
                    page_name: page_name.to_string(),
                    node_id: node_id.0,
                    node_name: node_name.to_string(),
                },
            );
        }
    }
    // Send (or Cmd+Enter) checks the recipient again now: an idle agent takes the prompt, a busy
    // one gets the confirmation first, and a gone one keeps the text in the field.
    let has_content = !app.compose.input.trim().is_empty() || !included.is_empty();
    if send && has_content && !app.compose.busy {
        let many = (cards.len() > 1).then(|| format!("{} cards", cards.len()));
        let scope = send_scope(!app.compose.input.trim().is_empty(), included.len(), many.as_deref().or(node.as_deref()).or(page_name.as_deref()));
        if app.compose.request_send(scope) {
            start_send(app, ctx);
        }
    }
    match confirm_modal(ctx, &mut app.compose) {
        Some(ConfirmOut::Cancel) => app.compose.confirm = None,
        Some(ConfirmOut::Send) => {
            app.compose.confirm = None;
            // The human confirmed a busy pane; it must still be there.
            if app.compose.send_check() == SendCheck::Unavailable {
                app.compose.notice = Some(UNAVAILABLE.to_string());
            } else {
                start_send(app, ctx);
            }
        }
        None => {}
    }
}

/// The names a draft records: the card and page the human was looking at.
struct Names<'a> {
    node: Option<&'a str>,
    page: Option<&'a str>,
    node_id: Option<NodeId>,
    page_id: Option<NodeId>,
    /// Every selected card and its name when there are several; empty for one card or none.
    cards: &'a [(NodeId, Option<String>)],
}

/// The context line with several cards selected: `COMMENTING ON · 3 cards · one comment for all`.
fn many_cards_context(count: usize) -> String {
    format!("COMMENTING ON · {count} cards · one comment for all")
}

/// What the human asked of the bar this frame.
#[derive(Default)]
struct BodyOut {
    /// Send was pressed (or Cmd+Enter).
    send: bool,
    /// `×  clear selection`: comment on the whole page again.
    clear: bool,
}

/// The bar's contents.
fn body(
    ui: &mut Ui,
    state: &mut State,
    context: &str,
    names: &Names<'_>,
    has_selection: bool,
    active: bool,
    included: &[usize],
    comments: usize,
) -> BodyOut {
    let mut out = BodyOut::default();
    // A draft may leave the list while its chip was active (sent, or removed elsewhere): never
    // keep editing one that is gone.
    if state.editing.is_some_and(|index| index >= state.drafts.len()) {
        state.stop_editing();
    }
    let editing = state.editing.and_then(|index| state.drafts.get(index).map(|draft| (index, draft)));
    let context_line = match editing {
        Some((index, draft)) => editing_context(draft, included, index, state.group_of(index).len()),
        None => context.to_string(),
    };
    // The context line is the accent in every state, as the design draws it.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.label(egui::RichText::new(&context_line).font(mono(8.5)).color(ACCENT()));
        if has_selection && names.node_id.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if text_action(ui, "×  clear selection", "Comment on the whole page") {
                    out.clear = true;
                }
                // A page comes from one card; with several selected it has no single source.
                if names.cards.len() < 2 && text_action(ui, "+ Page", "New page from this card") {
                    state.page_from_card = true;
                }
            });
        }
    });

    if !included.is_empty() {
        ui.add_space(6.0);
        draft_strip(ui, state, included, !has_selection);
    }
    ui.add_space(6.0);

    // Shift+Enter belongs to the multiline field. Consume Enter to save a comment and Cmd+Enter
    // to send the prompt with the saved comments in scope; neither inserts a newline.
    let input_focused = ui.is_enabled() && ui.memory(|m| m.has_focus(input_id()));
    let mut enter = false;
    let mut command_enter = false;
    if input_focused && !state.menu_open {
        ui.input_mut(|i| {
            i.events.retain_mut(|event| {
                let egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } = event else {
                    return true;
                };
                if modifiers.command {
                    command_enter = true;
                    false
                } else if modifiers.shift {
                    // The field gets the bare Enter it knows as a new line.
                    *event = egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
                    true
                } else if modifiers.any() {
                    true
                } else {
                    enter = true;
                    false
                }
            });
        });
    }
    // Esc stops editing and leaves the draft as it was; Backspace on an empty field does the
    // same, matching the design's "Esc or an empty field cancels" note. Esc is only read (not
    // consumed): it belongs to any popup or modal open over the map as well.
    let escape = state.editing.is_some() && !state.menu_open && ui.input(|i| i.key_pressed(Key::Escape));
    let backspace = !escape
        && state.editing.is_some()
        && state.input.is_empty()
        && input_focused
        && !state.menu_open
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace));
    if escape || backspace {
        state.stop_editing();
    }

    // The field spans the bar; the recipient chip and Send sit on the row under it, at the right.
    let placeholder = if names.cards.len() > 1 {
        "One comment for every selected card…"
    } else if names.node.is_some() {
        "Comment on this card and its children…"
    } else {
        "Comment on the whole page…"
    };
    let input_w = ui.available_width().max(90.0);
    // Grow with the prompt, up to eight rows; the full text stays reachable by scrolling.
    let per_line = ((input_w - 20.0) / 6.5).max(12.0);
    let rows = state.input.split('\n').map(|line| ((line.chars().count() as f32 / per_line).ceil() as usize).max(1)).sum::<usize>().clamp(1, 8);
    let editing_now = state.editing.is_some();
    let input_response = egui::Frame::NONE
        .fill(if editing_now { PANEL() } else { SOFT() })
        .stroke(Stroke::new(1.0, if editing_now { EDIT() } else { BORDER() }))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width((input_w - 22.0).max(40.0));
            ui.add(
                egui::TextEdit::multiline(&mut state.input)
                    .id(input_id())
                    .frame(egui::Frame::NONE)
                    .font(regular(12.5))
                    .hint_text(egui::RichText::new(placeholder).font(regular(12.5)).color(FAINT()))
                    .desired_rows(rows)
                    .desired_width(ui.available_width()),
            )
        })
        .inner;
    if state.focus_input {
        input_response.request_focus();
        state.focus_input = false;
    }

    // Why nothing is chosen, or why the last Send stayed: a line between the field and the row.
    if let Some((line, ink)) = help_line(state, active) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(line).font(regular(11.0)).color(ink));
    }
    ui.add_space(8.0);

    let mut send = false;
    // Laid out left to right with the widths known up front: a right-to-left layout would stretch
    // the auto-sized bar while it measures itself.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let can_send = active && state.target.is_some() && (!state.input.trim().is_empty() || !included.is_empty()) && !state.busy;
        let send_label = if state.busy {
            "Sending…"
        } else if matches!(state.send_check(), SendCheck::Confirm(_)) {
            "Review send"
        } else {
            "Send"
        };
        let send_w = send_width(ui, send_label);
        let chip_label = target_label(state, active);
        let chip_w = agent_menu::button_width(ui, &chip_label, CHIP_MIN_W, CHIP_MAX_W).min((ui.available_width() - send_w - 8.0).max(60.0));
        ui.add_space((ui.available_width() - chip_w - send_w - 8.0).max(0.0));
        let target_response = agent_menu::button(
            ui,
            Id::new("map_composer_target_chip"),
            &state.targets,
            state.target.as_ref(),
            &chip_label,
            vec2(chip_w, ROW_H),
            state.menu_open,
        );
        if target_response.clicked() {
            // The agent button opens the Agents modal (the agent picker page's One agents modal).
            state.open_menu();
        }
        ui.add_space(8.0);
        if send_button(ui, send_label, send_w, can_send) {
            send = true;
        }
    });

    if enter {
        state.save_comment(names);
        ui.ctx().request_repaint();
    }
    if command_enter {
        send = true;
    }
    // Sending a draft that is open in the field sends that draft: put the edit back into it and
    // empty the field, so the comment travels once (and not again as the prompt).
    if send {
        state.save_edited();
    }

    ui.add_space(8.0);
    let hint = if state.editing.is_some() {
        "⏎ save edit   ·   ⇧⏎ new line   ·   ⌘⏎ send all   ·   Esc stop editing".to_string()
    } else if !active {
        "connect a workspace to send · click connection status to retry · comments are kept".to_string()
    } else {
        // Esc hands the keys back to the canvas while the field has them.
        let esc = if input_focused { "esc back to the canvas   ·   " } else { "" };
        if names.cards.len() > 1 {
            format!("{esc}⏎ save   ·   ⌘⏎ send   ·   attaches to every selected card")
        } else {
            format!("{esc}⏎ save   ·   ⇧⏎ new line   ·   ⌘⏎ send all   ·   {comments} in the tree")
        }
    };
    ui.label(egui::RichText::new(hint).font(mono(8.5)).color(MUTED()));
    out.send = send;
    out
}

/// A quiet mono action at the right of the context line (`×  clear selection`, `+ Page`).
fn text_action(ui: &mut Ui, label: &str, tip: &str) -> bool {
    let size = vec2(text_width(ui, label, mono(8.5)), 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    a11y::button(&resp, label);
    let ink = if resp.hovered() { INK() } else { MUTED() };
    ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, mono(8.5), ink);
    resp.on_hover_text(tip).clicked()
}

/// The context line while a draft is open in the field: `EDITING DRAFT · on Card · 1 of 2`. A
/// comment saved on several cards (`cards` > 1) reads `on 3 cards`.
fn editing_context(draft: &Draft, included: &[usize], index: usize, cards: usize) -> String {
    let on = match (draft.node.as_deref(), draft.page.as_deref()) {
        _ if cards > 1 => format!("on {cards} cards"),
        (Some(node), _) => format!("on {node}"),
        (None, Some(page)) => format!("on {page}"),
        _ => "on the page".to_string(),
    };
    let position = included.iter().position(|queued| *queued == index).map_or(1, |at| at + 1);
    let total = included.len().max(position);
    format!("EDITING DRAFT  ·  {on}  ·  {position} of {total}")
}

/// Cards waiting with a draft comment carry an accent pin with their count at the top-right
/// corner, and glow blue while unselected, until a send carries them away ("send → a pin on each
/// card"). Several selected cards get an `N SELECTED` chip under them.
fn draft_marks(app: &FigApp, ctx: &egui::Context, page: Option<NodeId>, selection: &[NodeId]) {
    let Ok(doc) = app.session.doc() else { return };
    let mut counts: Vec<(NodeId, usize)> = Vec::new();
    for id in app.compose.drafts.iter().filter(|draft| draft.page_id == page).filter_map(|draft| draft.node_id) {
        match counts.iter_mut().find(|(card, _)| *card == id) {
            Some((_, count)) => *count += 1,
            None => counts.push((id, 1)),
        }
    }
    if counts.is_empty() && selection.len() < 2 {
        return;
    }
    let zoom = app.session.ds().map(|ds| ds.view.zoom).unwrap_or(1.0) as f32;
    let clip = app.canvas.rect;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::new("map_draft_glow"))).with_clip_rect(clip);
    let radius = (8.0 * zoom).clamp(0.0, 40.0) as u8;
    let screen = |id: NodeId| {
        let bounds = doc.world_bounds(id)?;
        let rect = Rect::from_min_max(app.to_screen(bounds.x0, bounds.y0), app.to_screen(bounds.x1, bounds.y1));
        rect.is_finite().then_some(rect)
    };
    for &(id, count) in &counts {
        let Some(rect) = screen(id) else { continue };
        if !rect.intersects(clip) {
            continue;
        }
        // An unselected card keeps the soft blue halo and pale blue edge; a selected one already
        // wears the selection's outline.
        if !selection.contains(&id) {
            for (grow, alpha) in [(6.0, 14u8), (4.0, 26), (2.5, 40), (1.0, 60)] {
                painter.rect_stroke(
                    rect.expand(grow),
                    CornerRadius::same(radius.saturating_add(grow as u8)),
                    Stroke::new(2.0, Color32::from_rgba_unmultiplied(0x2F, 0x6F, 0xDB, alpha)),
                    StrokeKind::Outside,
                );
            }
            painter.rect_stroke(rect, CornerRadius::same(radius), Stroke::new(1.0, Color32::from_rgb(0xA9, 0xC2, 0xEE)), StrokeKind::Inside);
        }
        let center = pin_center(rect);
        painter.circle_filled(center, PIN_R, ACCENT());
        painter.text(center, Align2::CENTER_CENTER, count.min(99).to_string(), mono(9.0), Color32::WHITE);
    }
    if selection.len() > 1 {
        let union = selection.iter().filter_map(|id| screen(*id)).reduce(|a, b| a.union(b));
        if let Some(union) = union {
            let label = format!("{} SELECTED", selection.len());
            let width = painter.layout_no_wrap(label.clone(), mono(7.5), ACCENT()).size().x + 20.0;
            let chip = Rect::from_min_size(pos2(union.min.x, union.max.y + 4.0), vec2(width.max(72.0), 18.0));
            painter.rect_filled(chip, CornerRadius::same(9), ACCENT_TINT());
            painter.text(pos2(chip.min.x + 10.0, chip.center().y), Align2::LEFT_CENTER, label, mono(7.5), ACCENT());
        }
    }
}

/// The draft pin's radius on screen: an 18 px disc.
const PIN_R: f32 = 9.0;

/// Where a card's draft pin sits: inside its top-right corner, as the design draws it; on a card
/// too small for the inset, at the corner itself.
fn pin_center(card: Rect) -> Pos2 {
    let inset_x = 22.0_f32.min(card.width() / 2.0);
    let inset_y = 20.0_f32.min(card.height() / 2.0);
    pos2(card.max.x - inset_x, card.min.y + inset_y)
}

/// The draft chips above the input: the first few, each editable and removable, then `+N`. On the
/// whole page they start folded into one `Whole page · N comments` chip, which opens them.
fn draft_strip(ui: &mut Ui, state: &mut State, included: &[usize], whole_page: bool) {
    if whole_page && !state.page_open && state.editing.is_none() {
        if page_summary_chip(ui, included.len()) {
            state.page_open = true;
        }
        return;
    }
    let mut action = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        let chips = strip_chips(&state.drafts, included);
        let visible = chips.len().min(VISIBLE_DRAFTS);
        for &index in chips.iter().take(visible) {
            let Some(draft) = state.drafts.get(index) else { continue };
            let cards = state.group_of(index).len();
            let label = if cards > 1 { same_comment_label(cards) } else { draft.text.replace('\n', " ") };
            if let Some(clicked) = draft_chip(ui, index, &label, state.editing == Some(index), cards > 1) {
                action = Some(clicked);
            }
        }
        let hidden = chips.len().saturating_sub(visible);
        if hidden > 0 {
            ui.label(egui::RichText::new(format!("+{hidden}")).font(mono(9.0)).color(MUTED()));
        }
    });
    match action {
        Some(DraftAction::Edit(index)) => state.toggle_editing(index, ui.ctx()),
        Some(DraftAction::Remove(index)) => state.remove_draft(index),
        None => {}
    }
}

/// The chips the strip shows for `included`: one per draft, except that drafts saved together on
/// several cards show once, at the first of them.
fn strip_chips(drafts: &[Draft], included: &[usize]) -> Vec<usize> {
    let mut seen = HashSet::new();
    included
        .iter()
        .copied()
        .filter(|index| match drafts.get(*index).and_then(|draft| draft.group) {
            Some(group) => seen.insert(group),
            None => true,
        })
        .collect()
}

/// A comment saved on several cards, as its chip reads: `Same comment · 3 cards`.
fn same_comment_label(cards: usize) -> String {
    format!("Same comment · {cards} cards")
}

/// `Whole page · 3 comments`: the page's drafts folded into one chip; a click opens them.
fn page_summary_chip(ui: &mut Ui, count: usize) -> bool {
    let label = format!("Whole page  ·  {count} comment{}", if count == 1 { "" } else { "s" });
    let width = (text_width(ui, &label, regular(10.5)) + 24.0).max(160.0);
    let (_, rect) = ui.allocate_space(vec2(width, 20.0));
    let resp = ui.interact(rect, page_summary_id(), Sense::click());
    a11y::button(&resp, &format!("{label}: show each comment"));
    ui.painter().rect_filled(rect, CornerRadius::same(6), if resp.hovered() { SOFT() } else { PANEL() });
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
    ui.painter().text(pos2(rect.min.x + 12.0, rect.center().y), Align2::LEFT_CENTER, &label, regular(10.5), INK());
    resp.on_hover_text("Show each comment").clicked()
}

/// `Page comment` above the bar while nothing is selected and the page has drafts: a pill with
/// the count that puts the keys in the field and opens the page's drafts there, so the thread is
/// read in the composer.
fn page_comment_chip(ctx: &egui::Context, bar: Rect, count: usize) -> bool {
    let label = "Page comment";
    let mut clicked = false;
    egui::Area::new(Id::new("map_composer_page_comment")).fixed_pos(pos2(bar.min.x, bar.min.y - 26.0 - 8.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            let width = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), INK()).size().x + 66.0;
            let (_, rect) = ui.allocate_space(vec2(width.max(150.0), 26.0));
            let resp = ui.interact(rect, page_comment_id(), Sense::click());
            a11y::button(&resp, &format!("{label}: {count} on the page"));
            ui.painter().rect_filled(rect, CornerRadius::same(13), if resp.hovered() { SOFT() } else { PANEL() });
            ui.painter().rect_stroke(rect, CornerRadius::same(13), Stroke::new(1.0, BORDER()), StrokeKind::Inside);
            ui.painter().text(pos2(rect.min.x + 14.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.0), INK());
            let badge = pos2(rect.max.x - 23.0, rect.center().y);
            ui.painter().circle_filled(badge, 9.0, ACCENT());
            ui.painter().text(badge, Align2::CENTER_CENTER, count.min(99).to_string(), mono(9.0), Color32::WHITE);
            clicked = resp.clicked();
        },
    );
    clicked
}

/// What a click on a draft chip asks for.
#[derive(Clone, Copy)]
enum DraftAction {
    /// Open the draft in the field (a second click on the active chip goes back to a new comment).
    Edit(usize),
    /// The × was pressed: delete the draft, and only this one.
    Remove(usize),
}

/// One draft chip: clicking its label opens the draft for editing; the × at its right removes it.
/// A comment on several cards (`many`) wears the accent tint.
fn draft_chip(ui: &mut Ui, index: usize, text: &str, active: bool, many: bool) -> Option<DraftAction> {
    let shown = widgets::elide_font(ui, text, 200.0, regular(10.5));
    let width = text_width(ui, &shown, regular(10.5)) + 33.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let x_rect = Rect::from_min_max(pos2(rect.max.x - DRAFT_X_W, rect.min.y), rect.max);
    let body_rect = Rect::from_min_max(rect.min, pos2(rect.max.x - DRAFT_X_W, rect.max.y));
    // The × is added last, so it sits above the label's click area and gets the click.
    let body = ui.interact(body_rect, draft_body_id(index), Sense::click());
    let close = ui.interact(x_rect, draft_remove_id(index), Sense::click());
    a11y::button(&body, &format!("Edit draft: {text}"));
    a11y::button(&close, &format!("Remove draft: {text}"));
    let close = close.on_hover_text("Remove");
    let hot = body.hovered() || close.hovered();
    let fill = match (active, many, hot) {
        (true, _, _) => EDIT_FILL(),
        (false, true, false) => ACCENT_TINT(),
        (false, true, true) => ACCENT_TINT_HOVER(),
        (false, false, true) => Color32::from_rgb(0xF1, 0xF0, 0xED),
        (false, false, false) => SOFT(),
    };
    // A pill: 20 pt tall, fully rounded.
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    let border = match (active, many) {
        (true, _) => EDIT(),
        (false, true) => ACCENT_TINT(),
        (false, false) => BORDER(),
    };
    ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = match (active, many) {
        (true, _) => EDIT(),
        (false, true) => ACCENT(),
        (false, false) => INK(),
    };
    ui.painter().text(pos2(rect.min.x + 9.0, rect.center().y), Align2::LEFT_CENTER, &shown, regular(10.5), ink);
    let x_color = if close.hovered() { ACCENT() } else { FAINT() };
    ui.painter().text(pos2(rect.max.x - 9.0, rect.center().y), Align2::RIGHT_CENTER, "×", regular(10.5), x_color);
    if close.clicked() {
        Some(DraftAction::Remove(index))
    } else if body.clicked() {
        Some(DraftAction::Edit(index))
    } else {
        None
    }
}

/// The line between the field and the row: why the last Send stayed, else why nobody is chosen.
fn help_line(state: &State, active: bool) -> Option<(String, Color32)> {
    if let Some(notice) = &state.notice {
        return Some((notice.clone(), DANGER()));
    }
    let agents = state.targets.iter().any(|info| !info.new_agent);
    (active && state.target.is_none() && agents).then(|| (NO_IDLE.to_string(), WORKING()))
}

/// The agent button's text: `Backend · Codex · p1 · idle`.
fn target_label(state: &State, active: bool) -> String {
    let nobody = if !active || (state.targets.is_empty() && state.targets_error.is_some()) { "No agents · reconnect" } else { "Choose an agent" };
    agent_menu::button_label(&state.targets, state.target.as_ref(), nobody)
}

/// Send's width: its least, or wider for `Review send`.
fn send_width(ui: &Ui, label: &str) -> f32 {
    (text_width(ui, label, regular(12.0)) + 24.0).max(SEND_W)
}

/// Send: the accent when it can send, grey when it can't. `Review send` when the chosen agent is
/// busy, so the human knows a confirmation comes first.
fn send_button(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> bool {
    let (_, rect) = ui.allocate_space(vec2(width, ROW_H));
    let resp = ui.interact(rect, Id::new("map_composer_send"), Sense::click());
    a11y::button(&resp, if label == "Send" { "Send the prompt" } else { label });
    let fill = if !enabled {
        DISABLED_FILL()
    } else if resp.hovered() {
        ACCENT_HOVER()
    } else {
        ACCENT()
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(12.0), if enabled { Color32::WHITE } else { FAINT() });
    resp.clicked() && enabled
}

/// What the confirmation's human asked for this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmOut {
    Cancel,
    Send,
}

/// The confirmation Send opens for a working or blocked agent. Cancel is the default and has the
/// keyboard; Esc or a click outside cancels. It reads the pane's status live, so a change while it
/// is open changes what it says; a pane that leaves can't be sent to from here.
fn confirm_modal(ctx: &egui::Context, state: &mut State) -> Option<ConfirmOut> {
    let confirm = state.confirm.as_ref()?;
    let line = confirm.target.line();
    let info = state.targets.iter().find(|info| info.target.line() == line).cloned();
    let scope = confirm.scope.clone();
    let focus_cancel = !confirm.focused;
    let ready = info.as_ref().map_or(Readiness::Unavailable, |info| readiness(&info.status));
    let (title, explanation) = match ready {
        Readiness::Working => (
            "Send to this working agent?",
            "This agent already has work in progress. A new prompt may change that work. Choose an idle agent to keep the tasks separate.",
        ),
        Readiness::Blocked => (
            "Send to this blocked agent?",
            "This agent is waiting for an answer or approval. Sending this prompt may not resolve what is blocking it.",
        ),
        Readiness::Ready => ("Send to this agent?", "This agent is idle again; the prompt goes straight to it."),
        Readiness::Unavailable => {
            ("This agent is no longer available", "The pane closed or its state can't be read. Nothing will be sent; choose an available agent.")
        }
    };
    let ink = match ready {
        Readiness::Working => WORKING(),
        Readiness::Blocked => DANGER(),
        Readiness::Ready => MAP_INK(),
        Readiness::Unavailable => MUTED(),
    };
    let width = (ctx.content_rect().width() - 32.0).clamp(240.0, 512.0);
    let mut out = None;
    let modal = egui::Modal::new(Id::new("map_composer_confirm"))
        .backdrop_color(Color32::from_black_alpha(40))
        .frame(
            egui::Frame::NONE.fill(PANEL()).stroke(Stroke::new(1.0, BORDER())).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(24)),
        )
        .show(ctx, |ui| {
            ui.set_width(width - 48.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add(egui::Label::new(egui::RichText::new(title).font(regular(22.0)).color(INK())).wrap());
            ui.add_space(14.0);
            if let Some(info) = &info {
                ui.add(egui::Label::new(egui::RichText::new(recipient(info, true)).font(regular(13.0)).color(ink)).wrap());
                let task = info.title.as_deref().map(str::trim).filter(|title| !title.is_empty());
                if let Some(task) = task {
                    ui.add_space(8.0);
                    let task = if ready == Readiness::Working { format!("Current task: {task}") } else { task.to_string() };
                    paragraph(ui, &task, 12.0, MUTED());
                }
            }
            ui.add_space(18.0);
            paragraph(ui, explanation, 13.0, INK());
            ui.add_space(24.0);
            paragraph(ui, &scope, 11.0, MUTED());
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                let (rect, cancel) = ui.allocate_exact_size(vec2(100.0, ROW_H), Sense::click());
                a11y::button(&cancel, "Cancel");
                if focus_cancel {
                    cancel.request_focus();
                }
                let ring = if cancel.has_focus() { INK() } else { BORDER() };
                ui.painter().rect_filled(rect, CornerRadius::same(6), if cancel.hovered() { SOFT() } else { PANEL() });
                ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, ring), StrokeKind::Inside);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Cancel", regular(11.5), INK());
                if cancel.clicked() {
                    out = Some(ConfirmOut::Cancel);
                }
                if let Some(info) = &info
                    && ready != Readiness::Unavailable
                {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = format!("Send to {}", info.label);
                        let w = (text_width(ui, &label, regular(11.5)) + 28.0).clamp(174.0, ui.available_width().max(174.0));
                        let (rect, send) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                        a11y::button(&send, &label);
                        ui.painter().rect_filled(rect, CornerRadius::same(6), if send.hovered() { ACCENT_HOVER() } else { ACCENT() });
                        let shown = widgets::elide_font(ui, &label, w - 20.0, regular(11.5));
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, &shown, regular(11.5), Color32::WHITE);
                        if send.clicked() {
                            out = Some(ConfirmOut::Send);
                        }
                    });
                }
            });
        });
    if let Some(confirm) = state.confirm.as_mut() {
        confirm.focused = true;
    }
    if out.is_none() && modal.should_close() {
        out = Some(ConfirmOut::Cancel);
    }
    out
}

/// What a send would carry, for the confirmation: `1 prompt + 2 drafts · Canvas and layers`.
fn send_scope(prompt: bool, drafts: usize, on: Option<&str>) -> String {
    let mut parts = Vec::new();
    if prompt {
        parts.push("1 prompt".to_string());
    }
    if drafts > 0 {
        parts.push(format!("{drafts} draft{}", if drafts == 1 { "" } else { "s" }));
    }
    let what = if parts.is_empty() { "Nothing yet".to_string() } else { parts.join(" + ") };
    match on {
        Some(on) => format!("{what} · {on}"),
        None => what,
    }
}

/// The map's right-click menu: Copy and Comment, and nothing from the editor.
pub fn context_menu(app: &mut FigApp, ctx: &egui::Context, pos: Pos2) {
    let copy = selection_text(app);
    let mut close = false;
    let area = egui::Area::new(Id::new("map_context_menu")).fixed_pos(pos).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(PANEL())
            .stroke(Stroke::new(1.0, BORDER()))
            .corner_radius(CornerRadius::same(8))
            .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(20) })
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.set_width(170.0);
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                if map_menu_item(ui, "Copy", "⌘C", !copy.is_empty(), false) {
                    if !copy.is_empty() {
                        crate::egui_copy_text(ctx, &copy);
                    }
                    close = true;
                }
                if map_menu_item(ui, "Comment", "", true, true) {
                    app.compose.focus_input = true;
                    ctx.request_repaint();
                    close = true;
                }
                let selected = app.session.selection().first().copied();
                let page = app.session.page().ok();
                if let Some(node_id) = selected {
                    let detail_page = app
                        .session
                        .doc()
                        .ok()
                        .and_then(|doc| doc.node(node_id))
                        .and_then(|node| node.plugin_data.get(crate::new_page::DETAIL_PAGE_KEY))
                        .and_then(|id| id.parse::<u64>().ok());
                    if let Some(detail_page) = detail_page
                        && map_menu_item(ui, "Open detail page", "", true, false)
                    {
                        app.run("page.select", json!({"id": detail_page}));
                        close = true;
                    }
                }
                if let Some(node_id) = selected
                    && let Some(page_id) = page
                    && let Some(page_name) = node_name(app, page_id)
                    && let Some(node_name) = node_name(app, node_id)
                    && map_menu_item(ui, "New page from card", "", true, false)
                {
                    let number = app.session.doc().map(|doc| doc.pages.len() + 1).unwrap_or(1);
                    app.new_page.start_component(number, crate::new_page::Source { page_id: page_id.0, page_name, node_id: node_id.0, node_name });
                    close = true;
                }
            });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    if close || outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.ui.context_menu = None;
    }
}

fn map_menu_item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, accent: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), if enabled { Sense::click() } else { Sense::hover() });
    a11y::button(&resp, label);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), SOFT());
    }
    let ink = if !enabled {
        FAINT()
    } else if accent {
        ACCENT()
    } else {
        INK()
    };
    ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.5), ink);
    if !shortcut.is_empty() {
        ui.painter().text(pos2(rect.max.x - 8.0, rect.center().y), Align2::RIGHT_CENTER, shortcut, mono(9.0), FAINT());
    }
    resp.clicked() && enabled
}

/// The selected cards' text, for Copy: every text layer under them, else their names.
fn selection_text(app: &FigApp) -> String {
    let Ok(doc) = app.session.doc() else { return String::new() };
    let ids = app.session.selection();
    let mut parts = Vec::new();
    for id in &ids {
        collect_text(doc, *id, &mut parts, 0);
    }
    if parts.is_empty() {
        for id in &ids {
            if let Some(node) = doc.node(*id)
                && !node.name.trim().is_empty()
            {
                parts.push(node.name.trim().to_string());
            }
        }
    }
    parts.join("\n").trim().to_string()
}

fn collect_text(doc: &figcraft_doc::Document, id: NodeId, parts: &mut Vec<String>, depth: u32) {
    if depth > 64 {
        return;
    }
    let Some(node) = doc.node(id) else { return };
    if let Some(text) = node.kind.text() {
        let characters = text.characters.trim();
        if !characters.is_empty() {
            parts.push(characters.to_string());
        }
    }
    for child in &node.children {
        collect_text(doc, *child, parts, depth + 1);
    }
}

/// The card's name (the kind's name when it is unnamed).
fn node_name(app: &FigApp, id: NodeId) -> Option<String> {
    let doc = app.session.doc().ok()?;
    let node = doc.node(id)?;
    Some(if node.name.trim().is_empty() { node.kind.type_name().to_lowercase() } else { node.name.trim().to_string() })
}

/// Force the next frame to re-read the agent list (the Agents modal's Refresh).
pub fn refresh_targets(app: &mut FigApp, ctx: &egui::Context) {
    app.compose.targets_at = 0.0;
    ensure_targets(app, ctx);
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_targets(app: &mut FigApp, ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else { return };
    let Ok(node) = workspaces::active_node(app) else { return };
    let identity = format!("{}:{}", node.address, node.session);
    let now = ctx.input(|input| input.time);
    let same = app.compose.targets_for.as_deref() == Some(identity.as_str());
    if same && now - app.compose.targets_at < 3.0 {
        return;
    }
    if same && app.compose.targets_rx.is_some() {
        return;
    }
    if !same {
        app.compose.targets.clear();
        app.compose.target = None;
        app.compose.targets_error = None;
    }
    app.compose.targets_for = Some(identity);
    app.compose.targets_at = now;
    let (tx, rx) = mpsc::channel();
    app.compose.targets_rx = Some(rx);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-compose-targets".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fetch_targets(&address, &workspace, &node.session)))
            .unwrap_or_else(|_| Err("The agent list request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.compose.targets_rx = None;
        app.compose.targets_error = Some(format!("Could not ask for the agents: {e}"));
        app.toast(format!("Could not ask for the agents: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn ensure_targets(app: &mut FigApp, _ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else { return };
    if app.compose.targets_for.as_deref() == Some(workspace.as_str()) {
        return;
    }
    app.compose.targets_for = Some(workspace);
    app.compose.targets_error = Some("The node is only available in the desktop app.".to_string());
}

fn poll_targets(app: &mut FigApp) {
    let received = app.compose.targets_rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.compose.targets_rx = None;
            app.compose.targets_error = Some("The agent list request ended without an answer.".to_string());
            app.toast("The agent list request ended without an answer.");
            return;
        }
    };
    app.compose.targets_rx = None;
    match received {
        Ok(targets) => {
            app.compose.targets = targets;
            app.compose.targets_error = None;
            let refreshed = refresh_target(app.compose.target.take(), &app.compose.targets);
            app.compose.target = refreshed.or_else(|| app.compose.page_default(app.compose.agent_page));
        }
        Err(error) => {
            app.compose.targets_error = Some(error.clone());
            app.toast(format!("Can't read the agents: {error}"));
        }
    }
}

/// The service's reply to `workspaces`: only what the composer reads.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Deserialize)]
struct TargetsReply {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    workspaces: Option<Vec<workspaces::Workspace>>,
    #[serde(default)]
    clis: Vec<String>,
}

/// Read the workspace's agents and the installed CLIs from the service.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_targets(address: &str, workspace: &str, session: &str) -> Result<Vec<TargetInfo>, String> {
    let line = ask(address, &json!({"method":"workspaces", "session":session}).to_string(), TARGETS_TIMEOUT)?;
    let reply: TargetsReply = serde_json::from_str(&line).map_err(|e| format!("The node sent an agent list this app can't read: {e}"))?;
    if !reply.ok {
        return Err(reply.error.unwrap_or_else(|| "The node refused the request.".to_string()));
    }
    let row = reply
        .workspaces
        .unwrap_or_default()
        .into_iter()
        .find(|row| row.id == workspace)
        .ok_or_else(|| "The workspace is no longer listed by the node.".to_string())?;
    Ok(agent_menu::targets_of(row.agents_detail, &reply.clis))
}

/// The recipient a page without a remembered agent starts on: the first idle agent in the list's
/// stable order. With none idle, nobody: the human chooses deliberately (or starts a new agent).
fn default_target(targets: &[TargetInfo]) -> Option<Target> {
    targets.iter().find(|info| !info.new_agent && info.status.trim().eq_ignore_ascii_case("idle")).map(|info| info.target.clone())
}

/// The recipient after the agent list refreshed. A chosen pane stays chosen whatever its status
/// does (its fresh status comes along); a pane that left the list stays chosen too, so Send can
/// say it is gone instead of quietly picking another. Only an empty choice takes the page's
/// default ([`State::page_default`]).
fn refresh_target(current: Option<Target>, targets: &[TargetInfo]) -> Option<Target> {
    match current {
        Some(current) => {
            let line = current.line();
            Some(targets.iter().find(|info| info.target.line() == line).map_or(current, |info| info.target.clone()))
        }
        None => None,
    }
}

/// How ready an agent with this Herdr status is to take a prompt.
pub fn readiness(status: &str) -> Readiness {
    match status.trim().to_ascii_lowercase().as_str() {
        "idle" | "done" | "ready" => Readiness::Ready,
        "working" | "busy" | "running" | "thinking" => Readiness::Working,
        "blocked" | "waiting" => Readiness::Blocked,
        _ => Readiness::Unavailable,
    }
}

/// The drafts a send from this selection would carry: the subtree of every selected card, the
/// page for no selection. Detached drafts on other cards and pages stay behind.
fn included_drafts(app: &FigApp, selection: &[NodeId], page: Option<NodeId>) -> Vec<usize> {
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let mut ids = HashSet::new();
    for root in selection {
        collect_subtree(doc, *root, &mut ids);
    }
    app.compose
        .drafts
        .iter()
        .enumerate()
        .filter(|(_, draft)| match selection.is_empty() {
            false => draft.node_id.is_some_and(|id| ids.contains(&id)),
            true => draft.page_id.is_some() && draft.page_id == page,
        })
        .map(|(index, _)| index)
        .collect()
}

fn collect_subtree(doc: &Document, id: NodeId, out: &mut HashSet<NodeId>) {
    if !out.insert(id) {
        return;
    }
    if let Some(node) = doc.node(id) {
        for child in &node.children {
            collect_subtree(doc, *child, out);
        }
    }
}

/// The `comments` a send carries: one entry per draft, so a comment saved on several cards
/// reaches the agent once on each card.
fn comment_payload(drafts: &[Draft]) -> Vec<Value> {
    drafts
        .iter()
        .map(|draft| {
            json!({
                "text": draft.text,
                "node": draft.node,
                "page": draft.page,
                "node_id": draft.node_id.map(|id| id.0),
                "page_id": draft.page_id.map(|id| id.0),
            })
        })
        .collect()
}

/// Selected cards a prompt names at most; the context packet stays short.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SELECTED: usize = 20;

/// Send the input as the prompt, with every draft attached, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start_send(app: &mut FigApp, ctx: &egui::Context) {
    if app.compose.busy {
        return;
    }
    if crate::workspaces::node_identity(app).is_some() && crate::workspaces::active_node(app).is_err() {
        crate::workspaces::reconnect(app);
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else {
        app.toast("Open a workspace to send comments. Your comments are kept.");
        return;
    };
    let Some(target) = app.compose.target.clone() else {
        app.toast("Choose an agent to send the comment to.");
        return;
    };
    let prompt = app.compose.input.trim().to_string();
    let selection = app.session.selection();
    let page = app.session.page().ok();
    let included = included_drafts(app, &selection, page);
    let sending: Vec<Draft> = included.iter().filter_map(|index| app.compose.drafts.get(*index).cloned()).collect();
    if prompt.is_empty() && sending.is_empty() {
        return;
    }
    let comments = comment_payload(&sending);
    let page_id = app.session.page().ok().map(|id| id.0);
    // The prompt is about the selected cards: name them, as each draft names its own.
    let selected: Vec<u64> = selection.iter().take(MAX_SELECTED).map(|id| id.0).collect();
    let Ok(node) = workspaces::active_node(app) else {
        app.toast("Open a connected node workspace first.");
        return;
    };
    let request = json!({
        "id": 1,
        "method": "send",
        "workspace": workspace,
        "session": node.session,
        "target": target.line(),
        "prompt": prompt,
        "comments": comments,
        "page_id": page_id,
        "selection": selected,
        "new_page": false,
    })
    .to_string();
    let (tx, rx) = mpsc::channel();
    // The page remembers the agent it sent to; so does a page with no history.
    app.compose.remember(page, target.clone());
    app.compose.sent_to = Some((page, target));
    app.compose.busy = true;
    app.compose.sending = sending;
    app.compose.rx = Some(rx);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-comment-send".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send_request(&address, &request)))
            .unwrap_or_else(|_| Err("The comment request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.compose.busy = false;
        app.compose.rx = None;
        app.compose.sending.clear();
        app.toast(format!("Could not send the comment: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_send(app: &mut FigApp, _ctx: &egui::Context) {
    app.toast("The node is only available in the desktop app.");
}

fn poll_send(app: &mut FigApp) {
    let received = app.compose.rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.compose.rx = None;
            app.compose.busy = false;
            app.compose.sending.clear();
            app.toast("The comment request ended without an answer.");
            return;
        }
    };
    app.compose.rx = None;
    app.compose.busy = false;
    let sent_to = app.compose.sent_to.take();
    match received {
        Ok(reply) if reply.ok => {
            if let Some((Some(page), sent)) = &sent_to {
                app.compose.prompted_on(*page, sent, reply.agent.as_ref());
            }
            if let (Some((page, sent)), Some(agent)) = (sent_to, reply.agent.as_ref()) {
                app.compose.started(page, &sent, agent);
            }
            let sent = if reply.sent > 0 { reply.sent } else { app.compose.sending.len() as u64 };
            let agent = reply
                .agent
                .as_ref()
                .and_then(agent_kind)
                .or_else(|| app.compose.target.as_ref().map(|target| kind_label(target.kind()).to_string()))
                .unwrap_or_else(|| "the agent".to_string());
            let sending = std::mem::take(&mut app.compose.sending);
            app.compose.drafts.retain(|draft| !sending.contains(draft));
            app.compose.input.clear();
            let what = if sent == 0 { "the prompt".to_string() } else { format!("{sent} comment{}", if sent == 1 { "" } else { "s" }) };
            app.toast(format!("Sent {what} to {agent}"));
        }
        Ok(reply) => {
            app.compose.sending.clear();
            app.toast(reply.error.unwrap_or_else(|| "The node refused the comment.".to_string()));
        }
        Err(error) => {
            app.compose.sending.clear();
            app.toast(error);
        }
    }
}

/// The agent's `kind` (or `name`) from the service's reply.
fn agent_kind(agent: &Value) -> Option<String> {
    ["kind", "name"]
        .into_iter()
        .filter_map(|key| agent.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn send_request(address: &str, request: &str) -> Result<SendReply, String> {
    let line = ask(address, request, SEND_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a reply this app can't read: {e}"))
}

/// Ask the node one newline-terminated JSON request and return its reply line.
#[cfg(not(target_arch = "wasm32"))]
fn ask(address: &str, request: &str, timeout: Duration) -> Result<String, String> {
    crate::node::ask(address, request, timeout)
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

pub(crate) fn input_id() -> Id {
    Id::new("map_composer_input")
}

/// The `Whole page · N comments` chip in the strip.
fn page_summary_id() -> Id {
    Id::new("map_composer_page_summary")
}

/// The `Page comment` pill above the bar.
fn page_comment_id() -> Id {
    Id::new("map_composer_page_comment_chip")
}

/// The clickable label half of a draft chip (it opens the draft in the field).
fn draft_body_id(index: usize) -> Id {
    Id::new(("map_composer_draft", index, false))
}

/// The × half of a draft chip (it removes the draft).
fn draft_remove_id(index: usize) -> Id {
    Id::new(("map_composer_draft", index, true))
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_menu::{agent_label, filter_agents};
    use crate::live_tests::{flat, has_text};
    use egui::{Event, PointerButton, RawInput, Shape};

    /// The composer's bar alone, drawn headless: its strip of drafts, its field, target and Send.
    struct Bar {
        ctx: egui::Context,
        state: State,
        /// The drafts a send from this selection would carry (all of them in these tests).
        included: Vec<usize>,
        time: f64,
        /// Nothing selected: the bar comments on the whole page.
        whole_page: bool,
        /// Several selected cards, each with its name; empty for one card.
        cards: Vec<(NodeId, Option<String>)>,
        /// The context line the bar is given.
        context: String,
    }

    impl Bar {
        /// A bar with `(card, text)` drafts.
        fn new(drafts: &[(&str, &str)]) -> Self {
            let mut state = State::default();
            for (node, text) in drafts {
                state.drafts.push(Draft {
                    text: (*text).to_string(),
                    node: Some((*node).to_string()),
                    page: Some("Page".to_string()),
                    node_id: None,
                    page_id: None,
                    group: None,
                });
            }
            let included = (0..state.drafts.len()).collect();
            let mut bar = Self {
                ctx: egui::Context::default(),
                state,
                included,
                time: 0.0,
                whole_page: false,
                cards: Vec::new(),
                context: "COMMENTING ON · Card · 0 comments".to_string(),
            };
            bar.frame(vec![]);
            bar
        }

        /// One frame of the bar: whether it asked to send, and what it painted.
        fn frame(&mut self, events: Vec<Event>) -> (bool, Vec<Shape>) {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let state = &mut self.state;
            let included = &self.included;
            let selected = !self.whole_page;
            let cards = &self.cards;
            let context = &self.context;
            let mut send = false;
            let mut out = self.ctx.run_ui(input, |ui| {
                ui.set_width(760.0);
                let names = Names {
                    node: selected.then_some("Card"),
                    page: Some("Page"),
                    node_id: selected.then_some(NodeId(42)),
                    page_id: Some(NodeId(7)),
                    cards,
                };
                send = body(ui, state, context, &names, selected, true, included, 0).send;
            });
            out.textures_delta.clear();
            (send, flat(&out.shapes))
        }

        fn rect(&self, id: Id) -> Rect {
            self.ctx.read_response(id).map_or_else(|| panic!("no widget {id:?}"), |response| response.rect)
        }

        /// A full press and release on `at`, then a quiet frame.
        fn click(&mut self, at: Pos2) {
            for pressed in [true, false] {
                self.frame(vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
                ]);
            }
            self.frame(vec![Event::PointerMoved(at)]);
        }

        fn click_chip(&mut self, index: usize) {
            let at = self.rect(draft_body_id(index)).center();
            self.click(at);
        }

        fn click_remove(&mut self, index: usize) {
            let at = self.rect(draft_remove_id(index)).center();
            self.click(at);
        }

        /// Where the field's caret is now, in characters.
        fn caret(&self) -> Option<usize> {
            egui::TextEdit::load_state(&self.ctx, input_id()).and_then(|state| state.cursor.char_range()).map(|range| range.primary.index.0)
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
            let (send, _) = self.frame(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
            send
        }
    }

    #[test]
    fn clicking_a_chip_loads_that_draft_into_the_field() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.input, "Fix the lane");
        assert_eq!(bar.state.editing, Some(0));
        assert!(!bar.state.drafts.is_empty(), "opening a draft does not send or remove it");

        bar.frame(vec![]);
        assert!(bar.ctx.memory(|m| m.has_focus(input_id())), "the field takes the keyboard");
        assert_eq!(bar.caret(), Some("Fix the lane".chars().count()), "the caret waits at the end");
        let (send, shapes) = bar.frame(vec![]);
        assert!(!send);
        assert!(has_text(&shapes, "Fix the lane"), "the chip stays in the strip, showing the draft's text");
    }

    #[test]
    fn the_remove_control_deletes_only_that_draft() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.editing, Some(0));

        bar.click_remove(0);
        assert_eq!(bar.state.drafts.len(), 1);
        assert_eq!(bar.state.drafts[0].text, "Merge agents");
        assert_eq!(bar.state.editing, None, "removing the edited draft ends the edit");
        assert!(bar.state.input.is_empty());

        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Merge agents"));
        assert!(!has_text(&shapes, "Fix the lane"));
    }

    #[test]
    fn a_second_click_on_the_active_chip_returns_to_a_new_comment() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents")]);
        bar.click_chip(0);
        assert_eq!(bar.state.editing, Some(0));
        bar.click_chip(0);
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty());
        assert_eq!(bar.state.drafts.len(), 2, "a second click must not delete the draft");
    }

    #[test]
    fn sending_while_editing_sends_the_edited_text_and_clears_editing() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.state.input = "Fix the lane: Lane Shared is gone".to_string();

        assert!(bar.key(Key::Enter, Modifiers::COMMAND), "Cmd+Enter asks for a send");
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty(), "the edit is not sent a second time as the prompt");
        assert_eq!(bar.state.drafts[0].text, "Fix the lane: Lane Shared is gone");
    }

    #[test]
    fn enter_saves_an_edit_without_sending_or_duplicating_it() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.state.input = "Fix the renamed lane".into();
        assert_eq!(bar.state.editing, Some(0));
        assert!(!bar.key(Key::Enter, Modifiers::NONE), "saving never asks for a send");
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty(), "the saved text leaves the field");
        assert_eq!(bar.state.drafts.len(), 1);
        assert_eq!(bar.state.drafts[0].text, "Fix the renamed lane");
        assert_eq!(bar.state.drafts[0].node.as_deref(), Some("Alpha"), "editing preserves the comment's context");
    }

    #[test]
    fn enter_saves_card_and_page_comments_without_a_recipient() {
        for whole_page in [false, true] {
            let mut bar = Bar::new(&[]);
            bar.whole_page = whole_page;
            bar.state.input = "  Keep this comment  ".into();
            bar.ctx.memory_mut(|m| m.request_focus(input_id()));
            assert!(!bar.key(Key::Enter, Modifiers::NONE));
            assert_eq!(bar.state.drafts.len(), 1);
            let draft = &bar.state.drafts[0];
            assert_eq!(draft.text, "Keep this comment");
            assert_eq!(draft.node_id, (!whole_page).then_some(NodeId(42)));
            assert_eq!(draft.node.as_deref(), (!whole_page).then_some("Card"));
            assert_eq!(draft.page_id, Some(NodeId(7)));
            assert_eq!(draft.page.as_deref(), Some("Page"));
            assert!(bar.state.input.is_empty());
            assert!(bar.state.confirm.is_none());
            assert!(bar.state.notice.is_none());
            bar.frame(vec![]);
            assert!(bar.ctx.memory(|m| m.has_focus(input_id())));
            assert!(!bar.key(Key::Enter, Modifiers::NONE), "empty Enter does nothing");
            assert_eq!(bar.state.drafts.len(), 1);
            bar.included = vec![0];
            assert!(bar.key(Key::Enter, Modifiers::COMMAND), "Cmd+Enter can send saved comments with an empty field");
            assert_eq!(bar.state.drafts.len(), 1, "comments stay until successful delivery");
        }
    }

    #[test]
    fn empty_enter_does_not_replace_an_open_comment() {
        let mut bar = Bar::new(&[("Alpha", "Keep this comment")]);
        bar.click_chip(0);
        bar.state.input = " \n ".into();
        assert!(!bar.key(Key::Enter, Modifiers::NONE));
        assert_eq!(bar.state.editing, Some(0));
        assert_eq!(bar.state.drafts[0].text, "Keep this comment");
    }

    #[test]
    fn saved_comments_follow_card_subtrees_and_page_scope() {
        let mut app = FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
        app.session.execute("file.new", &json!({})).unwrap();
        let page = app.session.page().unwrap();
        let card = NodeId(app.session.execute("frame.create", &json!({"name":"Card"})).unwrap()["id"].as_u64().unwrap());
        let child = NodeId(app.session.execute("shape.rectangle", &json!({"parent":card.0})).unwrap()["id"].as_u64().unwrap());
        let sibling = NodeId(app.session.execute("frame.create", &json!({"parent":page.0})).unwrap()["id"].as_u64().unwrap());
        let other = NodeId(app.session.execute("page.add", &json!({"name":"Other page"})).unwrap()["id"].as_u64().unwrap());
        for (node_id, page_id, text) in [
            (Some(card), page, "Card comment"),
            (Some(child), page, "Child comment"),
            (Some(sibling), page, "Sibling comment"),
            (None, page, "Page comment"),
            (None, other, "Other page comment"),
        ] {
            app.compose.input = text.into();
            app.compose.save_comment(&Names { node: None, page: None, node_id, page_id: Some(page_id), cards: &[] });
        }
        assert_eq!(included_drafts(&app, &[card], Some(page)), [0, 1]);
        assert_eq!(included_drafts(&app, &[], Some(page)), [0, 1, 2, 3]);
        assert_eq!(included_drafts(&app, &[], Some(other)), [4]);
    }

    /// A bar with three cards selected and their one comment typed in the field.
    fn three_cards() -> Bar {
        let mut bar = Bar::new(&[]);
        bar.cards = [(1, "Canvas and layers"), (2, "Components"), (3, "Sync")].map(|(id, name)| (NodeId(id), Some(name.to_string()))).to_vec();
        bar.context = many_cards_context(3);
        bar.state.input = "Same empty state, fix it once.".into();
        bar.ctx.memory_mut(|m| m.request_focus(input_id()));
        bar
    }

    #[test]
    fn several_selected_cards_share_one_comment_shown_as_one_chip() {
        let mut bar = three_cards();
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "COMMENTING ON · 3 cards · one comment for all"));
        assert!(has_text(&shapes, "esc back to the canvas   ·   ⏎ save   ·   ⌘⏎ send   ·   attaches to every selected card"));

        assert!(!bar.key(Key::Enter, Modifiers::NONE), "saving never asks for a send");
        assert_eq!(bar.state.drafts.len(), 3, "one draft per selected card");
        assert!(bar.state.input.is_empty());
        let group = bar.state.drafts[0].group;
        assert!(group.is_some());
        for (draft, (id, name)) in bar.state.drafts.iter().zip([(1, "Canvas and layers"), (2, "Components"), (3, "Sync")]) {
            assert_eq!(draft.text, "Same empty state, fix it once.");
            assert_eq!(draft.node_id, Some(NodeId(id)));
            assert_eq!(draft.node.as_deref(), Some(name));
            assert_eq!(draft.page_id, Some(NodeId(7)));
            assert_eq!(draft.group, group, "saved together, kept together");
        }

        bar.included = vec![0, 1, 2];
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Same comment · 3 cards"), "the three drafts show as one chip");
        assert!(!has_text(&shapes, "Same empty state, fix it once."), "the shared text is not repeated per card");
        assert_eq!(strip_chips(&bar.state.drafts, &bar.included), [0]);

        // Editing the chip edits every card's copy.
        bar.click_chip(0);
        assert_eq!(bar.state.editing, Some(0));
        bar.state.input = "Fix the empty state once, for all three.".into();
        bar.key(Key::Enter, Modifiers::NONE);
        assert!(bar.state.drafts.iter().all(|draft| draft.text == "Fix the empty state once, for all three."));

        // Its × removes all of them, and only them.
        bar.state.drafts.push(Draft { text: "Alone".into(), node: None, page: None, node_id: None, page_id: None, group: None });
        bar.included = vec![0, 1, 2, 3];
        bar.frame(vec![]);
        bar.click_remove(0);
        assert_eq!(bar.state.drafts.len(), 1);
        assert_eq!(bar.state.drafts[0].text, "Alone");
    }

    #[test]
    fn sending_with_several_cards_selected_carries_the_comment_on_each() {
        let mut app = FigApp::new(figcraft_engine::Session::new(), Box::new(crate::NoServices));
        app.session.execute("file.new", &json!({})).unwrap();
        let page = app.session.page().unwrap();
        let cards: Vec<NodeId> = ["Canvas and layers", "Components", "Sync", "Agents"]
            .iter()
            .map(|name| NodeId(app.session.execute("frame.create", &json!({"name":name})).unwrap()["id"].as_u64().unwrap()))
            .collect();
        let selected: Vec<(NodeId, Option<String>)> = cards[..3].iter().map(|id| (*id, node_name(&app, *id))).collect();
        app.compose.input = "Same empty state, fix it once.".into();
        app.compose.save_comment(&Names { node: None, page: Some("Page"), node_id: Some(cards[0]), page_id: Some(page), cards: &selected });
        app.compose.input = "Not this one".into();
        app.compose.save_comment(&Names { node: None, page: Some("Page"), node_id: Some(cards[3]), page_id: Some(page), cards: &[] });

        let included = included_drafts(&app, &cards[..3], Some(page));
        assert_eq!(included, [0, 1, 2], "the drafts on every selected card travel, not just the first");
        let sending: Vec<Draft> = included.iter().map(|index| app.compose.drafts[*index].clone()).collect();
        let payload = comment_payload(&sending);
        assert_eq!(payload.len(), 3, "one comment per card");
        for (comment, id) in payload.iter().zip(&cards[..3]) {
            assert_eq!(comment["text"], "Same empty state, fix it once.");
            assert_eq!(comment["node_id"], id.0);
            assert_eq!(comment["page_id"], page.0);
        }
        assert_eq!(payload[1]["node"], "Components");
    }

    #[test]
    fn the_draft_pin_sits_inside_the_top_right_corner() {
        let card = Rect::from_min_size(pos2(40.0, 105.0), vec2(260.0, 108.0));
        assert_eq!(pin_center(card), pos2(278.0, 125.0));
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0));
        assert_eq!(pin_center(tiny), pos2(5.0, 5.0));
    }

    #[test]
    fn enter_requires_the_composer_field_and_no_target_menu() {
        let mut bar = Bar::new(&[("Alpha", "Keep this draft")]);
        bar.state.input = "Unsent prompt".into();
        bar.ctx.memory_mut(|m| m.surrender_focus(input_id()));
        assert!(!bar.key(Key::Enter, Modifiers::NONE));
        assert_eq!(bar.state.input, "Unsent prompt");
        bar.ctx.memory_mut(|m| m.request_focus(input_id()));
        bar.state.menu_open = true;
        assert!(!bar.key(Key::Enter, Modifiers::NONE));
        assert_eq!(bar.state.drafts.len(), 1);
    }

    #[test]
    fn shift_enter_keeps_the_field_multiline() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.state.input = "Fix the lane: renamed".to_string();

        assert!(!bar.key(Key::Enter, Modifiers::SHIFT), "a new line is not a send");
        assert_eq!(bar.state.editing, Some(0), "the draft stays open");
        assert_eq!(bar.state.input, "Fix the lane\n: renamed", "Shift+Enter adds a line at the caret");
        assert_eq!(bar.state.drafts[0].text, "Fix the lane", "a new line does not save the draft");
    }

    #[test]
    fn escape_and_backspace_stop_editing_and_leave_the_draft() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.click_chip(0);
        bar.key(Key::Escape, Modifiers::NONE);
        assert_eq!(bar.state.editing, None);
        assert!(bar.state.input.is_empty());
        assert_eq!(bar.state.drafts[0].text, "Fix the lane");

        bar.click_chip(0);
        bar.state.input.clear();
        bar.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(bar.state.editing, None);
        assert_eq!(bar.state.drafts[0].text, "Fix the lane", "Backspace keeps the draft");
    }

    #[test]
    fn the_whole_page_folds_its_drafts_into_one_chip_that_opens_them() {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane"), ("Beta", "Merge agents"), ("Gamma", "Drop it")]);
        bar.whole_page = true;
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Whole page  ·  3 comments"));
        assert!(!has_text(&shapes, "Fix the lane"), "the page's drafts start folded");
        let at = bar.rect(page_summary_id()).center();
        bar.click(at);
        assert!(bar.state.page_open);
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Fix the lane") && has_text(&shapes, "Drop it"));
        assert_eq!(bar.state.drafts.len(), 3, "opening the chip removes nothing");
    }

    /// An agent row as the node lists it: `(pane, kind, status, name)`.
    fn agent(pane: &str, kind: &str, status: &str, name: Option<&str>) -> TargetInfo {
        TargetInfo {
            target: Target::Existing { pane: pane.into(), kind: kind.into(), status: status.into() },
            label: agent_label(name, None, pane),
            title: None,
            status: status.into(),
            focused: false,
            new_agent: false,
        }
    }

    fn cli(kind: &str) -> TargetInfo {
        TargetInfo {
            target: Target::New { kind: kind.into() },
            label: kind.into(),
            title: None,
            status: String::new(),
            focused: false,
            new_agent: true,
        }
    }

    fn pane(target: &Option<Target>) -> Option<String> {
        target.as_ref().map(Target::line)
    }

    #[test]
    fn the_default_recipient_is_the_first_idle_agent_and_nobody_without_one() {
        let mut focused = agent("w1:p1", "codex", "working", Some("Backend"));
        focused.focused = true;
        let targets =
            vec![focused, agent("w1:p2", "claude", "done", None), agent("w1:p3", "kimi", "idle", None), agent("w1:p4", "codex", "idle", None)];
        assert_eq!(pane(&default_target(&targets)).as_deref(), Some("pane:w1:p3"), "not the focused one, not done: the first idle");

        let busy = vec![agent("w1:p1", "codex", "working", None), agent("w1:p2", "kimi", "blocked", None), cli("claude")];
        assert_eq!(default_target(&busy), None, "with none idle nobody is chosen, not even a CLI");
        assert_eq!(default_target(&[]), None);
    }

    #[test]
    fn a_status_change_never_moves_the_chosen_pane() {
        let chosen = Some(agent("w1:p2", "claude", "idle", None).target);
        let later = vec![agent("w1:p1", "codex", "idle", None), agent("w1:p2", "claude", "working", None)];
        let refreshed = refresh_target(chosen.clone(), &later);
        assert_eq!(pane(&refreshed).as_deref(), Some("pane:w1:p2"));
        assert!(matches!(&refreshed, Some(Target::Existing { status, .. }) if status == "working"), "its fresh status comes along");

        let gone = vec![agent("w1:p1", "codex", "idle", None)];
        assert_eq!(pane(&refresh_target(chosen, &gone)).as_deref(), Some("pane:w1:p2"), "a pane that left stays chosen until Send says so");
        assert_eq!(refresh_target(None, &gone), None, "only an empty choice takes the page's default");
        let state = State { targets: gone, ..State::default() };
        assert_eq!(pane(&state.page_default(None)).as_deref(), Some("pane:w1:p1"));
    }

    #[test]
    fn a_page_is_being_designed_while_the_agent_it_was_sent_to_works() {
        let mut state = State { targets: vec![agent("w1:p5", "claude", "working", None)], ..State::default() };
        let page = NodeId(7);
        assert_eq!(state.designing(page), None, "never prompted");
        let sent = state.targets[0].target.clone();
        state.prompted_on(page, &sent, None);
        assert_eq!(state.designing(page).as_deref(), Some("Claude"));
        state.targets = vec![agent("w1:p5", "claude", "idle", None)];
        assert_eq!(state.designing(page), None, "done designing");
        // A new CLI's prompt follows the pane the node started for it.
        let other = NodeId(8);
        state.prompted_on(other, &Target::New { kind: "codex".into() }, Some(&json!({"pane": "w1:p9", "kind": "codex"})));
        state.targets.push(agent("w1:p9", "codex", "working", None));
        assert_eq!(state.designing(other).as_deref(), Some("Codex"));
    }

    #[test]
    fn each_page_remembers_its_agent_and_a_new_page_takes_the_last_one_used() {
        let (one, two, three) = (Some(NodeId(1)), Some(NodeId(2)), Some(NodeId(3)));
        let mut state = State {
            targets: vec![
                agent("w1:p1", "codex", "working", Some("Backend")),
                agent("w1:p2", "claude", "idle", Some("Tests")),
                agent("w1:p3", "kimi", "idle", Some("Docs")),
            ],
            ..State::default()
        };
        state.switch_page(one);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p2"), "no history anywhere: the first idle agent");

        state.choose(state.targets[2].target.clone());
        state.switch_page(two);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p3"), "a page with no history takes the last agent used anywhere");
        state.choose(state.targets[0].target.clone());
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p1"), "a busy agent can be picked; Send confirms it");

        state.switch_page(one);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p3"), "page one selects its own agent again");
        state.switch_page(two);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p1"), "and page two its own, busy or not");
        state.switch_page(three);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p1"), "a new page: the last agent used anywhere");

        // A remembered agent that left the list is skipped; with nobody idle, nobody is chosen.
        state.targets = vec![agent("w1:p3", "kimi", "working", Some("Docs")), cli("codex")];
        state.switch_page(two);
        assert_eq!(state.target, None, "page two's agent and the last one used are gone, and nobody is idle");
        state.switch_page(one);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p3"), "page one's agent is still here, working");

        // A status change on the same page never moves the choice.
        state.targets[0].status = "idle".into();
        state.switch_page(one);
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p3"));
    }

    #[test]
    fn a_new_agent_started_from_a_page_becomes_that_pages_agent() {
        let page = Some(NodeId(7));
        let mut state = State { targets: vec![cli("codex")], ..State::default() };
        state.switch_page(page);
        let new = Target::New { kind: "codex".into() };
        state.choose(new.clone());
        state.started(page, &new, &json!({"kind": "codex", "pane": "w1:p5", "prompted": true}));
        assert_eq!(pane(&state.target).as_deref(), Some("pane:w1:p5"), "the next send goes to the started pane, not to a second codex");
        assert_eq!(pane(&state.page_agents.get(&NodeId(7)).cloned()).as_deref(), Some("pane:w1:p5"));
        assert_eq!(pane(&state.last_used).as_deref(), Some("pane:w1:p5"));
    }

    #[test]
    fn send_rechecks_the_recipient_and_confirms_busy_agents() {
        let mut state = State {
            targets: vec![
                agent("w1:p1", "codex", "idle", None),
                agent("w1:p2", "claude", "working", None),
                agent("w1:p3", "kimi", "blocked", None),
                agent("w1:p4", "kimi", "unknown", None),
                cli("codex"),
            ],
            input: "Keep me".into(),
            ..State::default()
        };
        let pick = |state: &mut State, index: usize| state.target = Some(state.targets[index].target.clone());

        pick(&mut state, 0);
        assert_eq!(state.send_check(), SendCheck::Send);
        assert!(state.request_send("1 prompt".into()));
        assert!(state.confirm.is_none());

        pick(&mut state, 1);
        assert_eq!(state.send_check(), SendCheck::Confirm(Readiness::Working));
        assert!(!state.request_send("1 prompt + 2 drafts · Canvas".into()), "a working agent is never sent to straight away");
        let confirm = state.confirm.take().unwrap();
        assert_eq!(confirm.target.line(), "pane:w1:p2");
        assert_eq!(confirm.scope, "1 prompt + 2 drafts · Canvas");

        pick(&mut state, 2);
        assert_eq!(state.send_check(), SendCheck::Confirm(Readiness::Blocked));

        pick(&mut state, 3);
        assert!(!state.request_send(String::new()));
        assert!(state.confirm.is_none());
        assert_eq!(state.notice.as_deref(), Some(UNAVAILABLE), "an unreadable state never sends");

        state.target = Some(Target::Existing { pane: "w1:p9".into(), kind: "codex".into(), status: "idle".into() });
        assert_eq!(state.send_check(), SendCheck::Unavailable, "a vanished pane never sends, whatever it said before");

        pick(&mut state, 4);
        assert_eq!(state.send_check(), SendCheck::Send, "a new agent starts idle");
        state.target = None;
        assert_eq!(state.send_check(), SendCheck::Unavailable);
        assert_eq!(state.input, "Keep me", "no outcome of the check empties the field");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_pane_is_named_by_its_tab_not_by_its_session_title() {
        let detail = |pane: &str, tab: Option<&str>| workspaces::AgentDetail {
            pane: pane.into(),
            kind: "codex".into(),
            tab: tab.map(str::to_owned),
            title: Some("afterburn | Ready | Sync backend afterburn | main".into()),
            ..workspaces::AgentDetail::default()
        };
        let targets = agent_menu::targets_of(vec![detail("w1:p1", Some("small brain")), detail("w1:p4", None)], &[]);
        let labels: Vec<_> = targets.iter().map(|info| info.label.as_str()).collect();
        assert_eq!(labels, ["small brain", "Pane 4"]);
        assert!(targets.iter().all(|info| info.title.is_some()), "the title stays as the task");
    }

    #[test]
    fn an_agent_is_named_by_herdr_then_its_tab_then_its_pane() {
        assert_eq!(agent_label(Some("Backend"), Some("small brain"), "w1:p1"), "Backend");
        assert_eq!(agent_label(Some("  "), Some(" small\n brain "), "w1:p1"), "small brain");
        assert_eq!(agent_label(None, None, "w1:p4"), "Pane 4");
        assert_eq!(agent_label(None, None, "odd"), "Pane odd");
        let backend = agent("w1:p1", "codex", "idle", Some("Backend"));
        assert_eq!(recipient(&backend, false), "Backend · Codex · p1 · idle");
        assert_eq!(recipient(&agent("w1:p2", "claude", "", None), true), "Pane 2 · Claude · pane 2 · unknown");
    }

    #[test]
    fn the_menu_search_filters_by_label_kind_pane_and_status() {
        let targets = vec![
            agent("w1:p1", "codex", "idle", Some("Backend")),
            agent("w1:p2", "claude", "working", Some("UI review")),
            agent("w1:p3", "kimi", "blocked", Some("Docs")),
            cli("claude"),
        ];
        let labels = |query: &str| filter_agents(&targets, query).iter().map(|info| info.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(""), ["Backend", "UI review", "Docs"], "CLIs stay out of the list");
        assert_eq!(labels("review"), ["UI review"]);
        assert_eq!(labels("CLAUDE"), ["UI review"]);
        assert_eq!(labels("p3"), ["Docs"]);
        assert_eq!(labels("idle codex"), ["Backend"], "every word must match");
        assert!(labels("nothing").is_empty());
    }

    /// A bar with these agents and one draft to send.
    fn bar_with(targets: Vec<TargetInfo>) -> Bar {
        let mut bar = Bar::new(&[("Alpha", "Fix the lane")]);
        bar.state.targets = targets;
        bar.frame(vec![]);
        bar
    }

    #[test]
    fn the_agent_button_opens_the_agents_modal() {
        let mut bar = bar_with(vec![agent("w1:p1", "codex", "idle", Some("Backend"))]);
        bar.state.open_menu();
        assert!(bar.state.wants_agents, "the menu is the Agents modal now");
    }

    #[test]
    fn the_chip_names_the_recipient_and_send_reviews_a_busy_one() {
        let mut bar = bar_with(vec![agent("w1:p1", "codex", "idle", Some("Backend")), agent("w1:p2", "claude", "working", Some("UI review"))]);
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, NO_IDLE), "nothing is chosen until the list says who is idle");
        assert!(has_text(&shapes, "Choose an agent"));

        bar.state.target = Some(bar.state.targets[0].target.clone());
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Backend · p1 · idle"));
        assert!(has_text(&shapes, "Send") && !has_text(&shapes, "Review send"));
        assert!(!has_text(&shapes, NO_IDLE));

        bar.state.target = Some(bar.state.targets[1].target.clone());
        let (_, shapes) = bar.frame(vec![]);
        assert!(has_text(&shapes, "Review send"), "a working agent chosen explicitly asks for a review first");
    }

    /// One frame of the confirmation alone.
    fn confirm_frame(ctx: &egui::Context, state: &mut State, time: f64, events: Vec<Event>) -> (Option<ConfirmOut>, Vec<Shape>) {
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), time: Some(time), events, ..Default::default() };
        let mut out = None;
        let mut full = ctx.run_ui(input, |ui| out = confirm_modal(ui.ctx(), state));
        full.textures_delta.clear();
        (out, flat(&full.shapes))
    }

    #[test]
    fn the_confirmation_defaults_to_cancel_and_the_send_chord_never_confirms() {
        let mut working = agent("w1:p2", "claude", "working", Some("UI review"));
        working.title = Some("reviewing canvas navigation".into());
        let mut state = State { targets: vec![working], input: "Please check".into(), ..State::default() };
        state.target = Some(state.targets[0].target.clone());
        assert!(!state.request_send(send_scope(true, 2, Some("Canvas and layers"))));
        let ctx = egui::Context::default();

        // The modal sizes itself on its first frame and shows on the next.
        assert_eq!(confirm_frame(&ctx, &mut state, 0.05, vec![]).0, None);
        let (out, shapes) = confirm_frame(&ctx, &mut state, 0.1, vec![]);
        assert_eq!(out, None);
        for line in [
            "Send to this working agent?",
            "UI review · Claude · pane 2 · working",
            "Current task: reviewing canvas navigation",
            "1 prompt + 2 drafts · Canvas and layers",
            "Cancel",
            "Send to UI review",
        ] {
            assert!(has_text(&shapes, line), "missing {line:?}");
        }
        confirm_frame(&ctx, &mut state, 0.2, vec![]);
        assert!(ctx.memory(|m| m.focused().is_some()), "Cancel has the keyboard");

        let chord = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
        let (out, _) = confirm_frame(&ctx, &mut state, 0.3, vec![chord]);
        assert_ne!(out, Some(ConfirmOut::Send), "the send shortcut does not also confirm");

        let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let (out, _) = confirm_frame(&ctx, &mut state, 0.4, vec![escape]);
        assert_eq!(out, Some(ConfirmOut::Cancel));
        assert_eq!(state.input, "Please check", "Cancel keeps the text");

        // A blocked pane says so; a vanished one offers no send at all.
        state.targets[0].status = "blocked".into();
        let (_, shapes) = confirm_frame(&ctx, &mut state, 0.5, vec![]);
        assert!(has_text(&shapes, "Send to this blocked agent?"));
        state.targets.clear();
        let (_, shapes) = confirm_frame(&ctx, &mut state, 0.6, vec![]);
        assert!(!has_text(&shapes, "Send to UI review"));
    }
}
